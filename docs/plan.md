# weebo-operator-kit — plan de conception

Document de référence du kit commun aux opérateurs weebo
(`weebo-forgejo`, `weebo-authentik`, et les suivants). À lire avant tout
changement non trivial.

## Objectif

Les opérateurs weebo partagent la même architecture hexagonale et une
grande partie de leur plomberie : codes de raison stables, conditions de
status, allow-list par namespace, secret stores (Kubernetes + Vault),
leader election, webhook TLS, télémétrie, harnais envtest. Avant le kit,
chaque opérateur en portait une copie qui divergeait : forgejo a repris
celle d'authentik puis l'a fait évoluer (allow-list scopée, cibles de
secrets multiples, renouvellement du lease robuste…), sans retour vers
authentik.

Le kit porte **une** implémentation de cette plomberie ; un opérateur ne
garde que ce qui est propre à son système distant (CRD, gateway, use-cases
métier).

## Décisions

- **Repo dédié**, workspace Cargo, consommé par chaque opérateur en
  dépendance git épinglée sur un tag (`tag = "vX.Y.Z"`). Publication
  crates.io possible plus tard, une fois l'API stabilisée. Chaque
  opérateur garde son repo, son CI et son rythme de release.
- **Référence = forgejo, cas par cas** : quand les deux implémentations
  divergent, on part de celle de forgejo (la plus récente) et on reprend
  les apports d'authentik module par module.
- **Même découpage hexagonal que les opérateurs**, une crate par couche,
  avec les mêmes règles de dépendance :

| Crate                   | Couche       | Dépend de                     | Contenu                                                                                         |
|-------------------------|--------------|-------------------------------|-------------------------------------------------------------------------------------------------|
| `weebo-kit-domain`      | domain       | rien                          | `Reason`/`Severity`, `reason_codes!`, `Failure<R>`/`Advisory<R>`, moteur d'allow-list générique  |
| `weebo-kit-api`         | api          | serde, schemars, k8s-openapi  | sous-types de CRD : `Condition`, `SecretKeyRef`, `ObjectRef`, `SecretTarget`, `TlsOptions`…     |
| `weebo-kit-application` | application  | kit-api, k8s-openapi (types)  | ports `SecretStore`/`SecretStoreFactory`, helpers d'émission de secrets, `FakeSecretStore`       |
| `weebo-kit-adapters`    | outbound     | kit-application, kube, vaultrs| `K8sSecretStore`, `VaultSecretStore` (check-and-set) + `VaultLogins`, `RoutingSecretStore`, `read_secret_key` |
| `weebo-kit-runtime`     | inbound/bin  | kit-domain, kit-api, kit-application, kube | télémétrie, leader election + jauge, `Shutdown`, serveur webhook TLS, métriques, conditions + Events, status (`publish`, `record`), finalizer, reflectors, pod de l'opérateur |
| `weebo-kit-testkit`     | tests        | envtest, kube                 | `EnvTestCluster::start(crd_dir)`, `wait_for`/`wait_for_absence`                                 |
| `weebo-kit-perf`        | tests        | kit-testkit, kube             | `PerfRun` (binaire release contre envtest), `/proc`, métriques, `Budget::small_cluster`          |

## Ce qui va dans le kit (et ce qui n'y va pas)

Un module entre dans le kit quand :

1. il ne nomme aucun produit (Forgejo, Authentik…) ni aucune CRD
   concrète — ce qui varie est un paramètre (`field_manager`, préfixe de
   métriques, nom du lease, nom du CRD de policy) ou un type générique
   (`R: Reason`, `K` pour les kinds) ;
2. au moins deux opérateurs en ont l'usage, ou en auront l'usage sans
   réécriture.

Restent dans chaque opérateur : les `#[derive(CustomResource)]` (groupe,
kind, status propres), le catalogue `ReasonCode`, les gateways et leurs
ports, les use-cases `reconcile_*`, la factory qui lit l'instance CRD
(`ForgejoSecretStoreFactory` construit un `RoutingSecretStore` du kit à
partir de `ForgejoInstance.spec.secretStore`).

Cas particuliers tranchés :

- **Types CRD avec une valeur par défaut propre à l'opérateur** (ex.
  `VaultSecretStoreSpec.pathPrefix` par défaut `weebo-forgejo`) : restent
  dans l'opérateur, la valeur par défaut fait partie du schéma publié.
  L'adaptateur du kit prend une config neutre (`VaultConfig`).
- **Doc comments des types partagés** : ils deviennent les descriptions
  du schéma de chaque CRD — ils parlent de « l'instance » et de « l'objet
  distant », jamais d'un produit. Changer un doc comment du kit change les
  CRD générées de tous les opérateurs (`task recu` côté opérateur).
- **Codes de raison** : le kit ne définit aucun code. Le code générique
  (métriques, conditions) passe par le trait `Reason` ; le code qui doit
  produire une raison précise renvoie une erreur neutre
  (`SecretStoreError`…) que l'opérateur traduit (`SecretTargetFailed`).
- **Allow-list** : le moteur est générique sur le type des kinds et sur
  des « dimensions » de références nommées (`organizations`, `users` pour
  forgejo ; aucune pour authentik aujourd'hui). Un opérateur sans
  dimension garde la sémantique « namespace + kind ».

## Versioning et consommation

- SemVer sur le workspace entier (une version, un tag `vX.Y.Z`, via
  cocogitto). Tant que `0.x` : un bump mineur peut casser l'API.
- Les opérateurs dépendent du kit par
  `{ git = "https://github.com/batleforc/weebo-operator-kit", tag = "vX.Y.Z" }`
  dans leurs `[workspace.dependencies]`. Leur `deny.toml` doit alors
  autoriser cette source (`[sources] allow-git = [...]`).
- En développement local, un `path = "../weebo-operator-kit/crates/…"`
  (checkout voisin) permet d'itérer sur le kit et l'opérateur ensemble.
  **Ne jamais pousser un opérateur avec une dépendance `path`** : son CI
  et ses builds de conteneur ne voient pas le kit.
- Le kit épingle les mêmes versions de `kube`/`k8s-openapi` que les
  opérateurs : deux versions différentes lieraient deux copies de `kube`
  dont les types ne se correspondent pas. Monter `kube` = release du kit
  puis des opérateurs.
- Le paquet npm `@weebo/operator-docs` (`packages/operator-docs`) suit la
  version du workspace (`.hooks/bump-versions.sh`, hook `pre_bump` de
  cog ; `task docs:check` échoue sinon). Chaque tag `vX.Y.Z` attache
  `weebo-operator-docs-X.Y.Z.tgz` à sa release GitHub
  (`.github/workflows/release.yml`) ; le site de doc d'un opérateur en
  dépend par l'URL de ce tarball. Ses peers `fumadocs-*`/`next` sont
  épinglés : les monter = release du kit puis des sites. Même règle que
  `path` : un `file:` vers le checkout voisin (installé avec
  `npm install --install-links`) pour itérer, jamais poussé.

## État de la migration

### Itération 1 — kit + weebo-forgejo (faite)

- Kit créé avec les six crates ci-dessus, 22 tests unitaires + le
  contrat Vault (wiremock) déplacé depuis forgejo.
- weebo-forgejo migré : `ReasonCode` via `reason_codes!` + `Reason`,
  allow-list sur le moteur du kit, types CRD partagés ré-exportés,
  `Failure`/`Advisory` alias, ports et stores de secrets du kit,
  conditions/Events, métriques, `Shutdown`, télémétrie, leader election
  et serveur webhook du kit, testkit envtest/polling du kit. Seules les
  descriptions des CRD générées changent (texte neutre), pas les schémas.

### v0.2.0 — durcissement prod (cassant)

Issu de la revue « prod ready » de weebo-forgejo :

- **Propriété des secrets** : le port `SecretStore` prend un
  `SecretOwner { kind, namespace, name }` au lieu du namespace par défaut.
  `K8sSecretStore::new` prend en plus l'annotation propriétaire (ex.
  `forgejo.weebo.io/owner`) ; un Secret sans le label `managed-by` ou
  appartenant à un autre CR n'est jamais écrasé ni supprimé (les Secrets
  0.1.0, label seul, sont adoptés). Création par POST, puis `replace`
  gardé par `resourceVersion` (les clés retirées disparaissent). Vault :
  propriétaire dans le `custom_metadata` KV (best effort sans droit
  `metadata`), et un CR reste sous `<pathPrefix>/<namespace>/` (le sien,
  ou celui que nomme la cible d'un CR cluster), sauf chemin ouvert par
  les `pathGrants` de l'instance (`weebo_kit_domain::vault_paths`, RFC
  `docs/rfc-vault-path-grants.md`, kit v0.15.0).
- `emit::check_target(s)` : un CR namespacé n'écrit que dans son
  namespace (`TargetViolation`), vérifié avant tout appel et par les stores.
- `RoutingSecretStore::lazy` : Vault n'est contacté qu'à la première cible
  Vault ; `VaultLogins` verrouille par instance et rejoue un échec 15 s.
- **Leader election** réécrite (plus de `kube-leader-election`) : écritures
  du Lease gardées par `resourceVersion` (jamais deux leaders), expiration
  jugée sur l'horloge monotone locale uniquement (depuis v0.14.1, plus de
  raccourci sur `renewTime`) ; `leader::is_leader()` pour une jauge.
- `health` : `/readyz`, `/livez`, `/healthz` ; `webhook_server::spawn`
  prend `Health` + `Shutdown` et draine (`WEEBO_SHUTDOWN_DRAIN_SECONDS`).
- `telemetry::init` renvoie un `Telemetry` (flush à l'arrêt) ; Prometheus
  sur `WEEBO_METRICS_ADDR` (`serve_metrics`), logs JSON (`LOG_FORMAT=json`).
  OpenTelemetry 0.33.

### v0.3.0 — adoption des Secrets d'avant le label

- `K8sSecretStore` adopte aussi un Secret **sans label ni annotation**
  dont les `managedFields` montrent que le field manager de l'opérateur a
  écrit son `data` : les Secrets émis avant que l'opérateur ne pose le
  label `managed-by` (authentik ≤ 0.15, SSA nu) sont repris au lieu d'être
  vus comme étrangers.
- `VaultConfig::ca_source_of(secret_ref, pem)` : la `ca_source`
  (`namespace/name/key@sha256:…`) que chaque opérateur recalculait.
- Non cassant pour forgejo (ajouts seulement).

### Itération 2 — weebo-authentik (faite, sur le kit v0.3.0)

1. `testkit` (envtest/polling du kit, `testkit::envtest::start()` garde le
   chemin `deploy/crd/`), `telemetry`, `Shutdown`, serveur webhook (TLS
   rechargé, drain, désactivé sans certificat), `health`
   (`/readyz`/`/livez`, probes HTTPS du chart), leader election du kit
   (plus de `kube-leader-election`), jauge `weebo_authentik_leader`.
   OpenTelemetry 0.32 → 0.33.
2. `ReasonCode` via `reason_codes!` + `Reason` (catalogue inchangé) ;
   `Condition` du kit dans `AuthentikStatus` (schéma identique, seule la
   description de `reason` change).
3. `SecretKeyRef`, `TlsOptions`, `SecretStoreBackend` du kit ;
   `VaultConfig` construit depuis le `VaultSecretStoreSpec` d'authentik
   (défaut `pathPrefix: weebo-authentik` conservé côté opérateur).
   Le `secret_vault.rs` d'authentik n'avait rien de plus que celui du kit
   (qui ajoute timeout, propriétaire, re-login hors verrou, rejeu des
   échecs) : supprimé, avec `secret_k8s.rs`, `secret_fanout.rs` et leur
   contrat wiremock (couvert par celui du kit).
4. Port `SecretStore` du kit : `write_oauth2_credentials` devient
   `Oauth2Credentials::to_secret_data()` + `emit::write_all` ;
   `RoutingSecretStore::lazy` par instance. **CRD cassant** :
   `AuthentikApplication.spec.secretTargets` prend le `SecretTarget` du
   kit (`name` requis, `namespace` ajouté, `backend` par défaut
   `kubernetes`). Propriété des secrets : label
   `app.kubernetes.io/managed-by: weebo-authentik`, annotation
   `authentik.weebo.io/owner` ; un `path` Vault explicite doit rester
   sous `<pathPrefix>/<namespace>/`.
5. Allow-list : moteur du kit sans dimension. Changements de
   comportement : motifs `team-*`/`*` en glob, message de refus
   `no AuthentikNamespacePolicy rule allows <Kind> in namespace "<ns>"`.
6. `ReconcileMetrics` (`weebo_authentik_reconcile_*` conservés ; label
   `result` : `synced`/`errored` → `success`/`error`) et conditions du kit
   (`lastTransitionTime`, `observedGeneration`) + Event à chaque
   changement de `Ready` (RBAC `events.k8s.io` ajouté au chart).

### v0.4.0 — des Events pour chaque erreur et chaque changement d'état

- `weebo_kit_runtime::events`, après le patch de status :
  `announce_outcome` remplace `announce_ready_change` — un `Warning` à
  **chaque** reconcile finissant sur `Ready: False` (plus seulement aux
  transitions), un `Normal` quand `Ready` redevient `True` et pour chaque
  autre condition qui apparaît, change ou disparaît.
- `announce_change` : un `Normal` pour une étape que les conditions ne
  portent pas (objet distant créé, supprimé par le finalizer…).
- `announce_error`/`spawn_error` : les erreurs qui arrêtent un reconcile
  avant le status (appel API, finalizer, cleanup), depuis l'`error_policy`
  (actions `Reconcile`/`Cleanup`).
- Le `Recorder` de kube regroupe les répétitions en série : un seul
  `Recorder` partagé par opérateur.
- Le kit ne définit toujours aucun code : l'opérateur passe les siens
  (authentik : `KubernetesApiError`, `Created`, `Deleted` ajoutés).
- Additif : `announce_ready_change` reste (forgejo l'utilise encore).

### v0.5.0 — ce que coûte un opérateur (`weebo-kit-perf`)

But : pouvoir ajouter un opérateur sur un petit cluster sans se demander
ce qu'il va prendre. Inspiré du soak/profil de batlehub (#195), ramené à
ce qui compte pour un opérateur — la charge, ce sont des CR, pas des
requêtes HTTP.

- `PerfRun` lance le **binaire release** de l'opérateur contre envtest
  (sans webhook), attend le bail de leader (temps de démarrage), mesure
  chaque phase puis SIGTERM (temps d'arrêt).
- Par phase : RSS (médiane, pic, pente MiB/min), CPU (moyenne, pic, en
  millicores), descripteurs, threads (`/proc/<pid>`), réconciliations
  et erreurs, p95 de `<prefix>_reconcile_duration_seconds`, requêtes à
  l'apiserver de l'opérateur (`apiserver_request_total`, celles du harnais
  retirées), temps de convergence des objets.
- Forme du run, comme le soak de batlehub : warm-up → repos `baseline` →
  charges → repos `final`. Les fuites (RSS, fds, threads) se jugent entre
  les deux fenêtres **au repos**, la baseline prise après le warm-up.
- Le trafic propre de l'apiserver d'envtest (son bail d'identité, les
  endpoints `kubernetes`…) est mesuré avant le lancement de l'opérateur
  et retiré de chaque phase ; la ventilation `verbe ressource` des
  requêtes est dans le rapport (c'est elle qui dit d'où vient un dépassement).
- `Budget::small_cluster()` : repos < 64 MiB et 10 m, pic < 192 MiB,
  ≤ 60 req/min au repos (le bail de leader en coûte ~24), ≤ 20 req/objet,
  ≤ 50 ms de CPU/objet, convergence ≤ 60 s/100 objets, p95 ≤ 2 s,
  croissance RSS au repos ≤ 10 %, +16 fds, +4 threads. Chaque opérateur
  peut ajuster le sien.
- Le warm-up est un cycle complet de la taille de la charge mesurée :
  sinon le remplissage des pools (connexions, arènes) au cycle suivant se
  lit comme une fuite.
- Premier run (authentik, 200 `AuthentikGroup`) : 42 MiB au repos, ~20 ms
  de CPU et 9 req/objet, 200 objets convergés en ~1 s, aucune fuite.
  Forgejo (200 `ForgejoOrganization`) : 33 MiB au repos, 5-8 ms de CPU et
  6 req/objet. L'écart vient du squelette de controller : forgejo filtre
  sa watch (prédicats generation/annotations/finalizers) et réconcilie
  une fois par création, authentik deux (le patch de son finalizer et de
  son status relancent un reconcile), et relit le Secret du token à chaque
  réconciliation — argument de plus pour l'itération 3.
  Il a révélé deux défauts du kit, corrigés : les métriques exportées en
  `…_reconcile_total_total` / `…_duration_seconds_seconds` (l'exporter
  Prometheus ajoute lui-même `_total` et l'unité), et les buckets OTel par
  défaut (0, 5, 10… s) qui rangeaient toute réconciliation dans le
  premier et faisaient lire un p95 de ~5 s.
- Rapport Markdown + JSON dans `target/perf/`, comparaison report-only
  avec un JSON précédent (`WEEBO_PERF_BASELINE`), `assert_within` qui
  échoue au-delà du budget (`WEEBO_PERF_REPORT_ONLY=1` pour seulement
  rapporter).
- Pas encore : profil CPU (flamegraphs), recherche du point de rupture
  (le workflow de mesure régulière est arrivé en v0.8.0, ci-dessous).

### Itération 3 — généraliser l'application (faite, v0.6.0)

Ce qui était identique ou presque dans les deux opérateurs, rendu neutre
(un type de CRD, un accesseur de status, un domaine d'annotations, des
erreurs que l'opérateur traduit) :

- **Annotations communes** (`weebo-kit-api::annotations`) :
  `Annotations::new("<groupe>")` lit `force-orphan`,
  `allow-disruptive-update`, `rotate-secret`, `import-pending` sous le
  domaine de l'opérateur ; mêmes noms, même comportement partout.
- **Résolution des `*Ref`** (`weebo-kit-application::refs`) :
  `Referenced<Id>`, `RefState<Id>` (`Ready`/`NotFound`/`NotReady`/
  `OtherInstance`, plus `NotReconciled` depuis v0.14.0), `require` →
  `RefError` neutre (`describe` pour le message), `same_instance`
  (depuis v0.10.0 : égalité stricte des `BoundInstance` — kind, namespace,
  nom).
  La lecture des CR référencés reste à l'opérateur (ses kinds, son
  status) ; `ClusterError` et le port `PolicySource<R>` (générique sur la
  règle telle que la CRD de policy l'écrit) sont dans les ports du kit.
- **Admission** (`weebo-kit-runtime::admission`) : `router(admission,
  health, meter, prefix)` sert `/validate` + les sondes autour du trait
  `Admission` de l'opérateur (`admit(kind, namespace, object)` →
  `Denial { reason, message }`) : refus 403 avec le code de raison,
  échec fermé, `DELETE` et `UPDATE` sans changement de `spec` toujours
  acceptés (un CR dont le namespace a perdu sa policy n'est jamais
  bloqué en `Terminating`), compteur `<prefix>_webhook_decisions_total`.
  `typed::<K>()` pour l'objet typé, `handle()` pour les tests.
  `allow_list::evaluate_policies` fusionne les règles de toutes les
  policies.
- **Résolution de l'instance** (`weebo-kit-adapters::instance`) :
  `InstanceResolver<I>` (reflector `Store` ou API, `list`/`get`) et
  `select(instances, instance_ref)` → `SelectError` (`NotFound`/
  `NoInstance`/`Ambiguous`) que chaque opérateur mappe sur ses codes —
  forgejo `InstanceNotFound` pour 0 instance, authentik
  `AmbiguousDefaultInstance`, comme avant. Le squelette des gateway
  factories (client HTTP, cache par CA) reste dans les opérateurs : leurs
  besoins diffèrent (cache + sémaphore + métriques chez forgejo, aucun
  cache chez authentik).
- **Squelette de controller** (`weebo-kit-runtime::controller` et
  `status`) : `controller(api)` (watch filtrée par prédicats generation/
  annotations/labels/finalizers/suppression, concurrence
  `WEEBO_RECONCILE_CONCURRENCY`, 4 par défaut), `drive`, `jittered`,
  `requeue_after(steady, due_in)` (300 s/30 s ±20 %),
  `retry_after_error` (corps de l'`error_policy` : Event `Warning` +
  ~30 s), `finalize` (cleanup sauf `force-orphan`, puis Event du code
  `released` de l'opérateur), `status::publish` (merge patch des
  conditions + `observedGeneration` au succès + champs de l'opérateur,
  retenté sur erreur transitoire, puis Events). `Ctx`, l'enum `Error` et
  `needs_a_change` restent aux opérateurs.
- **Importer** (`weebo-kit-adapters::adopt`) : `Adopter { field_manager,
  annotations }.adopt(api, cr, remote_id, seed, bound_to, force)` — crée
  le CR en pause (`import-pending`), amorce son status, le relâche ;
  rejouable (déjà lié → ignoré, en pause → terminé), refuse un CR existant
  lié à rien sauf `--force`. `Summary`, `to_yaml`.

Migration : forgejo ne change pas de comportement (son code était la
source) ; ses compteurs de webhook sortent en
`weebo_forgejo_webhook_decisions_total` (plus de double `_total`).
Authentik gagne ce que forgejo avait déjà : prédicats et concurrence
bornée (une réconciliation par création au lieu de deux), gigue,
`force-orphan` (Event `Released`), la pause `import-pending`, le retry du
patch de status, le passe-droit `DELETE`/`UPDATE` sans `spec` du
webhook, et `importer --apply` qui adopte au lieu de demander un
`kubectl patch --subresource=status` à la main. Mesuré par
`weebo-kit-perf` (200 `AuthentikGroup`, même run qu'en v0.5.0) :
réconciliations 400 → 200 par rafale, CPU des charges −70 % (~7 ms par
objet), RSS au repos 42 → 32 MiB, descripteurs ~100 → 25, requêtes
apiserver 9 → 6 par création.

Reste propre à chaque opérateur : `RefKind`/`ReferencedCr` et le
`KubeClusterReader` (listes de CRD), `Governed` et les dimensions de
l'allow-list (organisations/utilisateurs chez forgejo), la règle des
admins de site, `Outcome`/`ReconcileOutcome`, les clés `key-owner` de
forgejo, la résolution par nom côté gateway chez authentik.

Depuis, authentik revérifie aussi la policy dans ses reconcilers, comme
forgejo, sans changement du kit : `KubePolicySource` implémente
`PolicySource<NamespaceRule>` (règles telles que l'écrit
`AuthentikNamespacePolicy`), lu par le webhook et par les reconcilers
d'`AuthentikApplication`/`AuthentikAccessPolicy` (un même `enforce`).
Refus → `Ready: False` `NamespaceNotAllowed`, aucun appel à Authentik,
rien de supprimé ; chaque changement de policy relance tous les CR de ces
deux kinds.

### Doc — `@weebo/operator-docs` (faite, publiée avec la release qui suit v0.6.0)

Les deux sites Fumadocs/Next portaient chacun leur copie du générateur de
pages CRD, du formulaire de manifeste, du thème et de l'échafaudage
`src/app/**`, sur des versions différentes (fumadocs 16.11/next 16.2 chez
authentik, 16.16/16.4 chez forgejo). Paquet `packages/operator-docs`,
sources TypeScript compilées par le site (`transpilePackages`) :

- **Générateur** `weebo-crd-docs` (bin, et `generateCrdDocs` en lib) :
  fusion des deux copies. Ce qui était propre à un opérateur passe dans
  son `docs/crd-docs.config.mjs` : exemples (toujours vérifiés contre le
  schéma), guides liés, ordre de la barre latérale, préfixe des kinds
  (nom des exemples), texte d'intro de l'index. Repris d'authentik : la
  première phrase qui ne coupe pas sur `i.e.`/`e.g.`, le gras retiré des
  descriptions, `crd-schemas/index.json` (la liste de kinds de
  `CrdBuilder`) ; de forgejo : la colonne des short names (seulement si
  un kind en a). Nouveau : le générateur vide lui-même ses sorties
  précédentes (un CRD supprimé ne laisse plus de page).
- **Formulaire de manifeste** (`crd-form`) : `CrdForm`, `CrdBuilder`,
  `fields`, `ChoicePicker`, utilitaires de schéma, thème YAML. La table
  des choix guidés (`CrdChoices`) devient une donnée de l'opérateur
  (`src/lib/crd-choices.ts`), passée par `docsMdxComponents(CHOICES)` ;
  la mécanique est celle d'authentik (`preset`, `require` en chemin
  pointé), un sur-ensemble de celle de forgejo.
- **Site** : `defineSite` (nom, phrase d'accroche, repo), `baseOptions`
  (marque, lien GitHub, thème), `DocsRootProvider` + `siteMetadata`,
  `createDocsPage`, `OperatorHome` (la page d'accueil « proof sheet »,
  avec les mots de l'opérateur), les handlers de recherche statique,
  `llms.txt`, `llms-full.txt`, Markdown par page et images OG,
  `withBasePath`, et `withOperatorDocs` pour tout `next.config.mjs`
  (export statique, MDX, `basePath` passé au code client). Les fichiers de
  route restent dans l'opérateur (Next lit `revalidate`/`dynamic` dans le
  fichier même), réduits à quelques lignes.
- **Thème** : `css/theme.css` (tokens BatleHub, pont Fumadocs, règles des
  composants, `@source` sur les composants du paquet) ; les polices
  restent dans le layout racine de l'opérateur (`next/font`), le thème les
  lit par `--font-silkscreen`/`--font-jetbrains-mono`. Le style des
  diagrammes (`diagram-style`) aussi.
- Versions alignées sur forgejo : fumadocs-core/ui 16.16.2, fumadocs-mdx
  15.4.6, next 16.4.0, react 19.3, en `peerDependencies` épinglées.
- `task docs:check` (version = workspace, `tsc` contre les peers, tests du
  générateur, dans la CI) et `task docs:pack` ; `task audit` audite les
  dépendances du paquet.

Migration : générées à nouveau, les pages `crds/*.mdx` et les schémas des
deux opérateurs sont identiques à l'octet près ; forgejo gagne
`crd-schemas/index.json`. Authentik monte en fumadocs 16.16/next 16.4 et
corrige au passage son lien « Edit on GitHub » (il oubliait le préfixe
`docs/`). Chaque opérateur ne garde que son contenu, sa config du
générateur, ses choix guidés, son layout racine (polices, `metadataBase`)
et ses diagrammes métier (`GroupSyncDiagram`/`TeamAccessDiagram` chez
forgejo).

Puis les pages que le kit rend communes, sous `content/docs/reference/`,
écrites par le même `weebo-crd-docs` (section `reference` de la config) et
vérifiées par la même étape de CI que les pages CRD : le texte est celui du
kit, écrit une fois dans le paquet ; l'opérateur ne fournit que ses
données, lues dans ses sources plutôt que recopiées.

- **Codes de raison** : le catalogue `reason_codes!` de l'opérateur
  (`crates/domain/src/error.rs`) est lu tel quel — codes, doc comments
  (colonne « signification », les renvois vers le code ou les notes de
  conception retirés), sévérités (le `match` de `fn severity`), code de
  succès. Où chaque code apparaît (`Ready`, condition propre, Event,
  refus d'admission) se déduit de la sévérité et d'un balayage texte des
  sources hors tests (`Denial::new`, `announce_change`,
  `controller::finalize`, paires `(Code, events::…)` de l'`error_policy`) :
  le `KubernetesApiError` d'authentik, qui n'atteint jamais un status, en
  sort « Event `Warning` avant tout status ». Colonne « que faire »
  optionnelle dans la config (forgejo y a reporté celle de son guide).
- **Events** : les règles d'`events::outcome_events` et du squelette de
  controller (Warning à chaque échec, Normal aux changements, erreurs
  avant status, `Cleanup`, libération et `force-orphan` sous le domaine du
  groupe des CRD), avec les codes de l'opérateur.
- **Métriques & santé** : `<préfixe>_reconcile_total`,
  `…_reconcile_duration_seconds` (buckets), `…_webhook_decisions_total`, la
  jauge `<préfixe>_leader` quand les sources la déclarent, les métriques
  propres de l'opérateur (config, noms vérifiés dans les sources), des
  requêtes PromQL, `/readyz`/`/livez` et les réglages du runtime
  (concurrence, requeue, bail, drain, TLS).
- **Budget de ressources** : `Budget::small_cluster()`, les surcharges de
  l'opérateur, et le dernier rapport `task perf` commité sous
  `docs/perf/` (mêmes calculs que `Report::check`, pire phase par limite).
- Les valeurs du kit citées par les pages (`small-cluster-budget.json`,
  `kit-runtime.json`) sont des copies : des tests de `weebo-kit-perf` et
  `weebo-kit-runtime` échouent si elles divergent des constantes Rust
  (`webhook_server::DEFAULT_PORT` ajouté pour ça).

Forgejo remplace par des liens les tableaux de son guide « Monitoring »
(codes de raison, métriques) et son paragraphe sur les Events ; authentik
n'avait rien d'équivalent. Rapports commités : forgejo mesuré sur le kit
v0.5.0, authentik sur v0.6.0.

### `cog.toml` commun (fait, publié avec la release qui suit v0.6.0)

`templates/cog.toml` est le `cog.toml` de chaque opérateur et du kit ;
seul le nom du repo (lien du changelog) varie. `templates/kit.yaml`,
copié dans le dossier de tâches de l'opérateur et inclus sous `kit:`,
le rend : `task kit:cog` lit le template au tag du kit qu'épingle le
`Cargo.toml` (`KIT_DIR=../weebo-operator-kit` pour un checkout voisin),
se met lui-même à jour et réinstalle les hooks. Le kit s'inclut avec
`KIT_DIR` = lui-même.

- `commit-msg` : `cog verify` (Conventional Commits).
- `pre-commit` : `task recu` s'il existe, `task lint`, les correcteurs de
  fins de ligne et d'espaces, `task test`, `task audit`, gitleaks.
- `pre_bump_hooks` (avant le tag) : `task audit`, `task test:all` (ou
  `task test` sans lui), puis `.hooks/bump-versions.sh` (versions du
  repo, `Cargo.lock` compris).
- Contrat de chaque repo : tâches `lint`, `test`, `audit` (`recu`,
  `test:all` facultatives), `.hooks/{bump-versions,end-of-line,
  whitespace-fixer}.sh`, `branch_whitelist = ["main"]`.

Authentik gagne au passage `task audit` (`deny.toml` repris de forgejo,
crates `publish = false`, h2 et rustls montés pour RUSTSEC-2026-0258/
-0285) et un job `audit` en CI ; son changelog pointait sur un autre repo.

### v0.8.0 — le budget de ressources mesuré chaque semaine

- `.github/workflows/perf.yml`, réutilisable (`workflow_call`) : chaque
  opérateur l'appelle sur un `schedule` (lundi 04:00 UTC) et à la main.
  Il lance `task perf` (binaire release contre envtest) comparé au
  rapport commité (`WEEBO_PERF_BASELINE`), publie le rapport (artefact +
  résumé du job), copie le JSON dans `docs/perf/<label>.json`, régénère la
  doc et pousse la branche `perf/refresh` avec une PR dont le corps est le
  comparatif Markdown. Le job échoue après publication quand le budget est
  dépassé : un dépassement se voit, sans perdre la mesure.
- Ouvrir la PR avec le `GITHUB_TOKEN` demande « Allow GitHub Actions to
  create and approve pull requests » ; sans ce réglage la branche est
  poussée et le résumé du job donne le lien pour ouvrir la PR. Une PR
  ouverte par ce token ne déclenche pas la CI (limite de GitHub).
- Le rapport `weebo-kit-perf` se date : `measured_at` (UTC) et
  `kit_version` ; la page « Resource budget » les affiche (le `perfNote`
  de l'opérateur n'ajoute plus que ce que le rapport ne dit pas), et
  `perfScheduled` y mentionne la mesure hebdomadaire.
- Les chiffres d'un runner GitHub ne sont pas ceux d'un nœud : ils
  servent à suivre une tendance d'une semaine à l'autre, le budget reste
  un garde-fou.

### Hors Rust (fait, publié avec la release qui suit v0.8.0)

Ce que les opérateurs recopiaient autour du code : chart, tâches, CI, et
le squelette d'un nouvel opérateur. Même règle que pour les crates : rien
ne nomme un produit, ce qui varie est un paramètre.

- **Library chart** `charts/weebo-operator-lib` (`type: library`, version
  = celle du workspace, poussée sur `oci://ghcr.io/batleforc/charts` par
  `release.yml`). Toutes les ressources d'un chart d'opérateur sauf les
  CRD : Deployment, ServiceAccount, ClusterRole(+Binding), Role du Lease,
  Role d'exemple namespacé, Service et `ValidatingWebhookConfiguration`
  du webhook, Issuer/Certificate cert-manager, PDB, Service de métriques,
  ServiceMonitor, PrometheusRule (alertes du kit sur ses métriques, noms
  dérivés du nom du chart), NetworkPolicy, health check Argo CD, en-tête
  de `NOTES.txt`. Référence = le chart de forgejo : mêmes noms de
  ressources, même contrat de values ; un chart d'opérateur se réduit à
  `templates/operator.yaml` (`weebo-operator.all`) + `_params.tpl`
  (groupe, kinds, kinds admis par le webhook, env, egress, règles et
  alertes propres). Vérifié : le chart forgejo réécrit ainsi rend les
  mêmes objets que l'actuel, à l'ordre d'une variable d'environnement et
  au texte des alertes près (son alerte sur l'API Forgejo passe en
  `params.alerts`).
- **Tâches partagées** `tasks/{rust,helm,repo}.yaml`, incluses à
  distance (Task ≥ 3.54) depuis
  `raw.githubusercontent.com/…/vX.Y.Z/tasks/…` et épinglées par
  `checksum:` (sha256, vérifié par Task ; `task --offline` tourne sur le
  cache) : `rust:{lint,audit,coverage,doctor}`, `helm:{deps,lint,crds}`
  (smoke renders du contrat de values, CRD enveloppées en templates
  `crds.install` + `keep`, l'ancien `sync-chart-crds.sh` de forgejo),
  `repo:{lint,audit}` (actionlint, hadolint, trivy config, npm audit). Le
  kit les inclut lui-même depuis son arbre.
- **Épinglage** : `templates/pin-kit.sh <repo> <tag> [kit]` aligne
  `Cargo.toml`, les includes distants et leurs checksums, les workflows
  réutilisables et la dépendance du chart sur une release ;
  `task kit:sync TAG=vX.Y.Z` (dans `templates/kit.yaml`) l'exécute puis
  `helm dependency update` et `kit:cog`. Le `pre_bump` du kit l'applique
  au gabarit avec les checksums de l'arbre tagué.
- **CI** : `.github/workflows/checks.yml` (`workflow_call`) — jobs
  `lint`, `test`, `audit` et `helm` (si `helm-task`), même contrat que
  `perf.yml` (`.github/actions/setup-mise` chez l'appelant). Les jobs
  propres (coverage contre le vrai système distant, images, fichiers
  régénérés) restent dans le workflow de l'opérateur. Le CI du kit
  l'appelle sur lui-même.
- **Gabarit `cargo-generate`** `templates/operator` :
  `cargo generate --git https://github.com/batleforc/weebo-operator-kit --tag vX.Y.Z templates/operator`
  (placeholders `product`, en CamelCase, et `license`, `Apache-2.0` par défaut). Workspace hexagonal câblé sur
  le kit (domain, api, application, adapters-outbound, adapters-inbound,
  operator, crdgen, testkit), CRD `<Product>Instance` (URL + token) et
  `<Product>Object` (objet distant d'exemple, finalizer), gateway HTTP
  à remplacer par le vrai client, tests unitaires + contrat wiremock +
  envtest ; chart sur la library chart, Taskfile sur les tâches
  partagées, CI sur `checks.yml`, `cog.toml` rendu par `task init`.
  `task template:check` (dans `test:all`, donc avant chaque tag, et en
  CI) le génère sur l'arbre du kit (crates et library chart par chemin),
  le compile, le lint, le teste et vérifie que ses CRD et son chart
  commités sont ceux qu'il régénère.

Reste à faire côté opérateurs : passer forgejo et authentik sur la
library chart (authentik gagne au passage le contrat de values de forgejo :
probes, drain, métriques, NetworkPolicy…), sur les tâches partagées et
sur `checks.yml`, puis supprimer leurs copies (`.tasks/helm.yaml`,
`deploy/scripts/sync-chart-crds.sh`, templates du chart). Le paquet
`weebo-operator-lib` de ghcr doit être rendu public une fois, à la
première publication.

### Périmètre de watch et mode namespacé (fait, publié en v0.10.0)

État en v0.9.0 : un opérateur = une installation pour tout le cluster.
Chaque controller surveille son kind avec `Api::all` (namespacés compris),
`InstanceResolver` liste les instances avec `Api::all`, le chart ne rend
qu'un `ClusterRole` (kinds, Secrets dans tout le cluster, Events). Le
cloisonnement entre équipes passe par la NamespacePolicy et le webhook,
pas par le périmètre de l'opérateur. Seuls le Lease (`POD_NAMESPACE`,
`Role`) et l'écriture de Secrets par un CR namespacé (son propre
namespace, `check_target`) sont déjà limités à un namespace.

Objectif : trois niveaux, cumulables, choisis à l'installation.

| Niveau                | Ce que l'opérateur voit                                  | Droits nécessaires                                                   |
|-----------------------|----------------------------------------------------------|----------------------------------------------------------------------|
| 1. Sélecteur de labels | seulement les CR qui portent les labels voulus           | inchangés (RBAC ne filtre pas par label)                             |
| 2. Un namespace       | kinds namespacés : un namespace ; kinds cluster : tout   | `Role` dans le namespace + `ClusterRole` réduit aux kinds cluster    |
| 3. Full namespacé     | un namespace, aucun kind cluster                         | `Role`/`RoleBinding` seulement (namespace surveillé + celui du Lease) |

#### Le périmètre dans le kit : `WatchScope`

- `weebo-kit-runtime::scope::WatchScope { namespace: Option<String>,
  labels: Option<String>, cluster_kinds: bool }`, lu au démarrage depuis
  `WEEBO_WATCH_NAMESPACE`, `WEEBO_WATCH_LABEL_SELECTOR` (syntaxe des
  sélecteurs Kubernetes, validée au démarrage) et
  `WEEBO_WATCH_CLUSTER_KINDS` (`true` par défaut). Rien de défini = le
  comportement actuel.
- `scope.api::<K>(client)` choisit l'`Api` d'après `K::Scope`
  (`NamespaceResourceScope` → `Api::namespaced`, `ClusterResourceScope` →
  `Api::all`) ; `scope.watcher_config()` porte le sélecteur de labels.
  Kind cluster demandé avec `cluster_kinds: false` → erreur au démarrage
  (jamais une boucle de 403 à l'exécution).
- `controller(api)` devient `controller(api, &scope)`, et
  `InstanceResolver`, les `watches` secondaires et les lectures des
  `ClusterReader` des opérateurs passent par le même scope. **Cassant**
  pour les opérateurs (signature) : ils remplacent leurs `Api::all`.
- Dès la v1 : tout le cluster (rien de défini) ou un seul namespace
  (D3). Plusieurs namespaces plus tard : `kube` ne
  surveille qu'un namespace ou tous, et fusionner plusieurs watchers dans
  un même reflector casse son `Init`/`InitDone` (le dernier namespace
  initialisé remplace le store) ; il faudra un `Controller` par namespace,
  `Ctx` partagé, concurrence par controller.

#### 1. Sélecteur de labels

- Filtrage côté apiserver (`watcher::Config::labels`) : moins de mémoire
  et d'événements, utile aussi pour le budget de `weebo-kit-perf`.
- **Un seul périmètre pour tout ce que l'opérateur voit** : le sélecteur
  s'applique à tous ses kinds (instance comprise). Un objet hors
  sélection est invisible : une référence vers lui donne `NotFound`
  (l'instance : `InstanceNotFound`). Les lectures par nom (`get`) ne
  filtrent pas par label côté serveur : le kit revérifie le sélecteur
  sur l'objet lu.
- **Pas une frontière de sécurité** : RBAC ne filtre pas par label, le
  `ClusterRole`/`Role` donne accès à tous les objets du kind. Le
  sélecteur sert à répartir le travail (deux releases pour deux serveurs
  distants, un shard par équipe), pas à cloisonner.
- Retirer le label d'un CR le fait sortir du périmètre : le watcher le
  voit comme supprimé, son finalizer reste, l'objet distant n'est plus
  géré, et sa suppression reste bloquée en `Terminating`. Garde-fou : le
  webhook (quand il est actif) refuse un `UPDATE` qui fait sortir du
  sélecteur un objet portant le finalizer de l'opérateur, sauf avec
  `force-orphan`. Passer un objet d'un shard à l'autre : changer le
  label en une seule écriture (même finalizer, même `remoteId` en
  status, l'autre release le reprend).
- Le chart dérive `webhook.objectSelector` du même sélecteur, sauf
  valeur explicite.
- Deux releases dont les périmètres se recouvrent réconcilient les mêmes
  CR (même finalizer, même field manager : rien ne le détecte). Règle
  documentée : périmètres disjoints, et au plus une release sans
  sélecteur ni namespace.

#### 2. Un namespace

- Les kinds namespacés sont surveillés dans `WEEBO_WATCH_NAMESPACE`
  seulement ; les kinds cluster (instance, NamespacePolicy, kinds
  cluster de l'opérateur) restent surveillés partout.
- RBAC rendu par le chart : `Role` + `RoleBinding` dans le namespace
  surveillé pour les kinds namespacés (+ `status`, `finalizers`), les
  Secrets et les Events ; `ClusterRole` réduit aux kinds cluster. Les
  Secrets restent dans le `ClusterRole` tant qu'un kind cluster émet des
  Secrets dans un namespace de son choix (`SecretTarget.namespace`).
- Events d'un objet cluster : le `Recorder` de `kube` les publie dans
  `default` → droit `events` dans `default` (aujourd'hui couvert par le
  `ClusterRole`).
- Lease : déjà dans `POD_NAMESPACE`, nom dérivé de la release : deux
  releases dans deux namespaces ne se gênent pas.
- Les CR namespacés d'un opérateur limité à un namespace ne se lient
  qu'à une `LocalInstance` de ce namespace (D2) : la NamespacePolicy n'a
  plus rien à gouverner (règle 4 ci-dessous). L'`Instance` cluster sert
  aux seuls kinds cluster.

#### 3. Full namespacé — prérequis

But : une release installable par un admin de namespace, sans aucun
objet ni droit cluster. Prérequis, dans l'ordre :

1. **CRD installées à part.** Une CRD est un objet cluster : un admin du
   cluster les installe (`crds.install: false` dans la release, bundle
   `crds.yaml` de la release GitHub). Plusieurs releases partagent les
   mêmes CRD : elles doivent tourner sur des versions compatibles avec
   le schéma installé ; monter une CRD reste une opération d'admin.
2. **Aucun kind cluster démarré.** `WEEBO_WATCH_CLUSTER_KINDS=false` :
   chaque opérateur déclare le scope de ses kinds (déduit de `K::Scope`),
   ne démarre que les controllers namespacés, le chart ne rend que leur
   RBAC. Un kind cluster nécessaire au démarrage → erreur explicite.
3. **Une instance namespacée.** Le kind instance actuel est cluster ; il
   faut un second kind, namespacé : `<Product>LocalInstance` (D1),
   `<Product>Instance` reste le kind cluster, inchangé :
   - même spec que l'instance cluster, sauf ses références de Secrets :
     `LocalSecretKeyRef` (son namespace) au lieu de `SecretKeyRef` ;
   - utilisable par les CR de son namespace ; depuis un autre namespace
     ou par un CR cluster seulement si elle l'accepte (D5) ;
   - Vault : la release partage l'identité Vault de l'opérateur. Une
     instance namespacée ne choisit ni mount ni rôle d'auth au-delà de
     ce que la config de la release autorise, et ses chemins sont forcés
     sous `<pathPrefix>/<namespace>/` — sinon un locataire lit les
     chemins des autres avec l'identité de l'opérateur ;
   - kit : `InstanceResolver<I, L>` (cluster + locale) et résolution
     d'un CR namespacé selon le périmètre (D2) ; `refs::same_instance`
     compare `(kind, namespace, name)` et non plus un nom (**cassant**).
     Pas de nouveau cas d'ambiguïté : l'ordre de résolution tranche.
4. **NamespacePolicy optionnelle.** Règle : un CR lié à une instance
   cluster reste soumis à la policy ; un CR lié à une instance
   namespacée ne l'est pas (le token de son instance, fourni par le
   namespace, borne ce qu'il peut faire chez le distant ; les dimensions
   d'allow-list comme les organisations de forgejo ne s'appliquent
   plus). Kit : `PolicySource` désactivable (`policies::Unrestricted`),
   choisi par instance et pas globalement, pour qu'un mode de niveau 2
   ne perde jamais la policy par erreur. Sans kind cluster, le
   controller de policy n'est pas démarré et sa CRD n'a pas besoin
   d'être installée.
5. **Webhook optionnel.** La `ValidatingWebhookConfiguration` est un
   objet cluster : `webhook.enabled` (nouveau, `true` par défaut) ;
   `false` → pas de `ValidatingWebhookConfiguration` rendue (le
   Service et le certificat restent : le serveur HTTPS porte aussi les
   sondes, et ce sont des objets namespacés), les
   refus passent par les reconcilers seuls (forgejo et authentik
   revérifient déjà la policy dans leurs reconcilers) : on perd le
   refus immédiat au `kubectl apply`, pas la règle. Alternative : un
   admin du cluster installe la configuration une fois, avec un
   `namespaceSelector` sur ce namespace.
6. **Références namespacées seulement.** Un kind namespacé ne peut
   référencer qu'un kind namespacé (ou un nom distant brut) : sinon il
   lui faut lire un kind cluster. À traiter dans chaque opérateur :

   | Opérateur | Kinds namespacés                                                          | Références vers un kind cluster                                                     | À prévoir                                                                 |
   |-----------|---------------------------------------------------------------------------|-------------------------------------------------------------------------------------|---------------------------------------------------------------------------|
   | forgejo   | Repository, Webhook, DeployKey, OAuth2Application, AccessToken            | Organization, User, Team (owner, teams, token, OAuth2), UserKey cluster             | variantes namespacées de User/Organization/Team, ou propriétaire par nom distant |
   | authentik | Application, AccessPolicy                                                 | aucune vers un CR cluster hors instance (`outpostRef`, `groupRef` : noms distants)   | instance namespacée seulement                                             |

7. **Chart sans objet cluster.** Pas de `ClusterRole(Binding)`, pas de
   `ValidatingWebhookConfiguration`, pas de ConfigMap de santé Argo CD
   (namespace d'Argo) ; restent Deployment, ServiceAccount, Roles,
   Service, Issuer/Certificate, PDB, ServiceMonitor, PrometheusRule,
   NetworkPolicy. `helm template` d'un mode full namespacé vérifié par
   `helm:lint` : aucun objet sans namespace dans le rendu.
8. **Preuve par les tests.** Un test envtest lance le binaire avec un
   ServiceAccount lié seulement aux `Role` rendus (impersonation dans le
   client de test) : un appel cluster → 403 → le test échoue. À
   vérifier : le mode d'autorisation de l'apiserver d'envtest (RBAC
   doit être actif, sinon le lancer avec `--authorization-mode=RBAC`).
   Dans le gabarit (`template:check`) puis dans chaque opérateur.

L'importer reste hors périmètre : il crée des CR cluster ; une variante
namespacée viendra avec les kinds namespacés de chaque opérateur.

#### Values du chart

```yaml
watch:
  namespace: ""        # "" = tout le cluster ; nom = ce namespace (souvent celui de la release)
  labelSelector: {}    # un LabelSelector (matchLabels / matchExpressions), ex. {matchLabels: {weebo.io/shard: a}}
  clusterKinds: true   # false = full namespacé (prérequis ci-dessus)
webhook:
  enabled: true        # false = refus par les reconcilers seuls
```

`labelSelector` est un objet `LabelSelector` (validé par le schéma des
values) plutôt qu'une chaîne : le chart le passe tel quel en
`objectSelector` des webhooks et en JSON à l'opérateur
(`WEEBO_WATCH_LABEL_SELECTOR`), sans conversion en syntaxe `a=b,c in
(…)` dans un template Helm. `clusterKinds: false` exige `watch.namespace` et `crds.install: false`
(`fail` au rendu sinon). Mêmes noms de ressources qu'aujourd'hui quand
rien n'est défini : un opérateur qui ne touche pas `watch` rend le même
chart.

#### Décisions (prises le 2026-10-12)

- **D1 — nom de l'instance namespacée** : `<Product>Instance` reste le
  kind cluster (non cassant), `<Product>LocalInstance` est la variante
  namespacée.
- **D2 — résolution de l'`instanceRef` sans `kind`** (référence par nom
  seul, cas des CR existants), selon le périmètre de la release :
  - **opérateur limité à un namespace** (`watch.namespace` défini) : la
    `LocalInstance` de ce nom dans le namespace du CR, seulement ; vide
    → la seule `LocalInstance` du namespace. L'`Instance` cluster n'est
    jamais candidate, même au niveau 2 ;
  - **opérateur sur tout le cluster** : l'`Instance` cluster de ce nom
    d'abord, puis la `LocalInstance` de ce nom dans le namespace du CR ;
    vide → la seule `Instance`, sinon la seule `LocalInstance` du
    namespace. Même nom des deux côtés : l'`Instance` l'emporte.

  Conséquence à traiter : une `Instance` créée après coup avec le nom
  d'une `LocalInstance` déjà utilisée capterait ses CR (autre serveur
  distant). Garde-fou : le status du CR enregistre l'instance résolue
  (`kind` + nom) à la liaison ; si la résolution change ensuite, le CR
  passe `Ready: False` avec un Event, l'opérateur ne suit pas le
  changement (même logique que `OtherInstance`). Préciser le `kind` (D5)
  supprime ce cas. Un CR cluster sans `kind` ne se lie qu'à une
  `Instance`.
- **D5 — `instanceRef` typée** : la référence peut préciser le kind visé
  et, pour une `LocalInstance`, son namespace :

  ```yaml
  instanceRef:
    name: main
    kind: LocalInstance   # Instance | LocalInstance ; absent → résolution D2
    namespace: team-a     # LocalInstance seulement ; absent → celui du CR
  ```

  - type neutre dans `weebo-kit-api` : `InstanceRef { name, kind:
    Option<InstanceKind>, namespace: Option<String> }`,
    `InstanceKind::{Instance, LocalInstance}` (valeurs courtes, pas le
    kind complet `<Product>LocalInstance`, pour rester sans nom de
    produit) ; doc comments sur « l'instance ». Le status enregistre la
    référence résolue complète (`kind`, `namespace`, `name`) et
    `same_instance` compare ce triplet ;
  - validations (CEL dans le schéma quand c'est possible, sinon webhook
    et reconciler, raison neutre que l'opérateur traduit) :
    `namespace` interdit avec `kind: Instance` ; obligatoire pour un CR
    cluster qui vise une `LocalInstance` (il n'a pas de namespace par
    défaut) ;
  - **référence vers un autre namespace = emprunt des credentials d'un
    autre locataire** (le token de la `LocalInstance` est le sien). Elle
    n'est acceptée que si la `LocalInstance` y consent :
    `spec.sharedWith { namespaces: [...], namespaceSelector,
    clusterScoped: bool }`, vide par défaut (son namespace seulement),
    sur le modèle du `ReferenceGrant` de Gateway API. Refus →
    `Ready: False`, aucun appel au distant. Ce consentement tient lieu
    de NamespacePolicy pour ces CR (règle 4) ;
  - opérateur limité à un namespace : `kind: Instance` ou un
    `namespace` différent du sien → `Ready: False` (il ne voit ni l'un
    ni l'autre, et n'en a pas les droits) ; le webhook le refuse dès
    l'admission. Viser un autre namespace demande une release sur tout
    le cluster (lecture des `LocalInstance` et de leurs Secrets
    ailleurs) ;
  - **schéma : `instanceRef` devient un objet** (décidé). Aujourd'hui
    c'est une chaîne (optionnelle chez forgejo, requise chez authentik) ;
    un schéma structurel ne peut pas accepter « chaîne ou objet », donc
    les CR stockés ne se relisent plus (désérialisation de l'opérateur
    en échec, mise à jour refusée) tant qu'ils ne sont pas réécrits ;
  - **migration hors ligne, sans webhook de conversion.** Un webhook de
    conversion est déclaré dans la CRD et pointe vers un seul Service :
    incompatible avec plusieurs releases namespacées qui partagent les
    CRD, et un objet cluster de plus. Le changement se fait dans
    `v1alpha1` (alpha : le schéma peut casser), dans cet ordre :
    1. arrêter l'opérateur (`replicas: 0`) — les objets distants ne
       sont pas touchés, seules les réconciliations s'arrêtent ;
    2. appliquer les nouvelles CRD ;
    3. réécrire chaque CR : `spec.instanceRef: "main"` →
       `spec.instanceRef: { name: main }` (sans `kind` : la résolution
       D2 garde le comportement d'avant). Outil générique du kit
       (`weebo-kit-adapters::migrate`, sur `DynamicObject`, liste de
       kinds en paramètre, rejouable, `--dry-run`), exposé par la
       sous-commande `importer migrate-instance-ref` de chaque
       opérateur ;
    4. redémarrer l'opérateur.

    Les manifests GitOps qui portent ces CR doivent être réécrits dans la
    même release, sinon Argo CD réapplique la chaîne (refusée par le
    nouveau schéma). La note de release de chaque opérateur porte la
    procédure ; c'est une release cassante.
- **D3 — périmètre de la v1** : tout le cluster et un seul namespace
  dès la première release ; plusieurs namespaces plus tard (un
  `Controller` par namespace).
- **D4 — garde-fou du label retiré** : oui, dans le webhook du kit
  (refus d'un `UPDATE` qui fait sortir du sélecteur un objet portant le
  finalizer, sauf `force-orphan`).

#### Ordre de réalisation

1. Kit : `WatchScope` (tout le cluster ou un namespace, + labels),
   `controller(api, &scope)`, `InstanceResolver` sur le scope, garde-fou
   du label dans l'admission (D4), chart (`watch.*`, RBAC par mode,
   `webhook.enabled`), gabarit, test envtest RBAC. Release mineure
   (cassante).
2. Kit + gabarit : `LocalInstance` (D1), résolution par périmètre et
   instance enregistrée au status (D2), référence typée et
   `sharedWith` (D5), outil de migration de `instanceRef`,
   `same_instance` typé,
   policy désactivable par instance, mode `clusterKinds: false`.
   Release mineure.
3. Opérateurs : authentik d'abord (aucune référence cluster hors
   instance), puis forgejo avec ses kinds namespacés (User/Organization/
   Team), chacun dans sa propre release ; la première qui adopte l'objet
   `instanceRef` porte la migration hors ligne et la réécriture des
   manifests GitOps.

#### État (2026-10-12)

**Étape 1 — faite, publiée en v0.10.0** (`82d8f66` ; `task lint`,
`task test:all` dont `template:check`, `task docs:check` verts) :

- `weebo-kit-runtime::scope` : `WatchScope` (`cluster()`,
  `namespace(ns)`, `with_labels(LabelSelector)`, `from_env()` sur
  `WEEBO_WATCH_NAMESPACE`/`WEEBO_WATCH_LABEL_SELECTOR` — JSON, invalide
  → échec au démarrage), `api::<K>()` choisi par le trait `KindScope`
  sur `K::Scope` (namespacé → `Api::namespaced` si un namespace est
  surveillé, cluster → `Api::all`), `watcher_config()`, `list_params()`,
  `selects()`/`selects_labels()` pour revérifier un `get`.
- `controller(api, &scope)` : **cassant** (signature).
- `weebo-kit-runtime::admission::{ScopeGuard, scope_router}` (D4) :
  `POST /scope` refuse un `UPDATE` qui fait sortir du sélecteur un objet
  portant le finalizer, sauf `force-orphan` ; la raison est un code de
  l'opérateur (gabarit : `LeavesWatchScope`). Indépendant de
  `Admission` : un opérateur sans `/validate` sert quand même `/scope`.
- `InstanceResolver::with_selector(Option<Selector>)` (non cassant) :
  liste filtrée côté serveur, `get` revérifié. Pas de `WatchScope` dans
  `weebo-kit-adapters` (il ne dépend pas du runtime) : le binaire passe
  `scope.selector()`.
- `weebo-kit-testkit` : `EnvTestCluster::client_as_service_account(ns,
  name)` (impersonation ; l'apiserver d'envtest autorise bien en RBAC,
  vérifié dans controller-runtime v0.24.1).
- Chart : `watch.namespace`/`watch.labelSelector` → env ;
  `watchRole` (`<fullname>-watch`, kinds namespacés + Secrets + Events
  dans le namespace surveillé) et `ClusterRole` réduit aux kinds cluster
  (Secrets et Events y restent) ; webhook `scope.<group>` (UPDATE de
  tous les kinds, `/scope`) avec un sélecteur ; `namespaceSelector`/
  `objectSelector` des webhooks par défaut sur le scope ;
  `webhook.enabled`. NOTES et README. Smoke renders dans
  `tasks/helm.yaml` (donc aussi chez chaque opérateur au prochain
  `kit:sync`).
- Gabarit : `Ctx.scope`, controllers et reflector d'instances sur le
  scope, `webhook::router(health, scope)` sert `/scope`, code
  `LeavesWatchScope`, values + schéma. Test envtest `watch_scope.rs` :
  controller limité à `team-a` + `shard=a`, compte de service avec le
  RBAC du mode namespace seulement — l'objet du scope est réconcilié,
  ceux d'un autre shard ou d'un autre namespace ne sont pas touchés,
  aucune erreur de watch. Vérifié qu'il échoue avec `WatchScope::cluster()`
  (403 sur la liste cluster-wide).
- Doc (`@weebo/operator-docs`) : les deux variables dans « Runtime
  settings ».

Écarts et limites connus :

- Le test envtest recopie les règles RBAC du chart (`kindRules`) au
  lieu de rendre le chart : à garder alignés à la main.
- Le garde-fou ne voit que ce que l'apiserver lui envoie : sans webhook
  (`webhook.enabled: false`) ou s'il est indisponible avec
  `failurePolicy: Ignore`, retirer le label reste possible.
- Les opérateurs forgejo et authentik ne sont pas migrés (étape 3) :
  leurs `Api::all` dans les `watches` secondaires et leurs
  `ClusterReader` ne connaissent pas encore le scope ; la release du
  kit casse leur build (`controller(api, &scope)`).
- Pas de bump de version ni de tag : release mineure à faire (cassante).

**Étape 2 — faite, publiée en v0.10.0** (même release
que l'étape 1 ; `task lint`, `task test:all` dont `template:check`,
`task docs:check` verts) :

- `weebo-kit-api::instance` : `InstanceRef { name, kind?, namespace? }`
  (D5, objet ; règle CEL « `namespace` seulement avec `LocalInstance` »
  dans le schéma), `InstanceKind::{Instance, LocalInstance}`,
  `BoundInstance { kind, namespace?, name }` (enregistrée au status,
  `governed_by_policy()` vrai seulement pour une `Instance`),
  `SharedWith { namespaces, namespaceSelector, clusterScoped }`, trait
  `SharesItself` (implémenté par la `LocalInstance` de l'opérateur).
- `weebo-kit-application::instances` : `resolve()` pure (D2 selon le
  périmètre, D5, consentement `sharedWith` avec les labels du namespace
  pour un `namespaceSelector` — opérateur inconnu = refus),
  `check_binding()` (le garde-fou d'instance résolue : un `instanceRef`
  sans `kind` dont le nom résout maintenant vers une autre instance →
  `BindingChanged`, rien n'est suivi ; préciser `kind` ou changer de nom
  relie), trait `InstanceDirectory`, erreurs neutres `ResolveError` /
  `InstanceError`.
- `refs` : `Referenced.instance: Option<BoundInstance>`, `RefState::of(
  found, &ours)`, `same_instance(&BoundInstance, &BoundInstance)`
  (synchrone, compare le triplet), `RefError::describe(kind, name)` —
  **cassant**. Un CR référencé pas encore lié est `NotReady`, plus
  `OtherInstance`.
- `SecretStoreFactory::store_for(&BoundInstance)` — **cassant** :
  l'instance est résolue avant.
- `weebo-kit-adapters::instances::Instances<I, L>` : l'annuaire des deux
  kinds (stores ou API, sélecteur, namespace surveillé, sans `I` quand
  `cluster_kinds` est faux), `get(&BoundInstance)` pour les gateway
  factories ; ne lit un `Namespace` (droit `get namespaces`) que pour un
  partage par sélecteur. `InstanceResolver`/`select`/`SelectError`
  restent pour les opérateurs pas encore migrés.
- `weebo-kit-adapters::migrate` : `migrate_instance_refs(client, kinds,
  field_manager, dry_run)` (merge patch `"main"` → `{name: main}`,
  rejouable) ; gabarit : `<binaire> migrate-instance-ref [--dry-run]`.
  Test envtest : CRD remise à l'ancien schéma, CR écrit avec la chaîne,
  nouvelle CRD, migration, relu typé.
- `WatchScope::without_cluster_kinds()`, `WEEBO_WATCH_CLUSTER_KINDS`,
  `cluster_kinds()`, `ensure::<K>()` (un kind cluster sans kinds
  cluster → erreur au démarrage).
- Chart : `watch.clusterKinds` (env ; `fail` sans `watch.namespace` ou
  avec `crds.install`) ; sans kinds cluster ni `ClusterRole(Binding)`
  ni `ValidatingWebhookConfiguration` (smoke render : aucun objet
  cluster dans le rendu) ; `get namespaces` dans le `ClusterRole` d'un
  opérateur sur tout le cluster (partage par sélecteur) ; NOTES.
- Gabarit : `<Product>LocalInstance` (namespacé, `LocalSecretKeyRef`,
  `sharedWith`) et son controller ; `ObjectSpec.instanceRef:
  Option<InstanceRef>` ; `status.instance` ; résolution dans le
  controller d'objet (cleanup sur l'instance enregistrée) ; gateway
  factory sur `Instances` ; codes `InstanceAmbiguous`,
  `InstanceNotAllowed`, `InstanceBindingChanged` ; binaire : réflecteurs
  des deux kinds, controller d'`Instance` seulement avec kinds cluster.
  Tests envtest : niveau 2 (objet lié à la `LocalInstance`, jamais à
  l'`Instance` homonyme ; `kind: Instance` → `InstanceNotAllowed`) et
  full namespacé (un `Role` seulement, aucune lecture cluster possible,
  `LocalInstance` et objet `Ready`).
- Doc : `WEEBO_WATCH_CLUSTER_KINDS` dans « Runtime settings ».

Pas fait à l'étape 2, à reprendre dans les opérateurs (étape 3) :

- **Confinement Vault d'une `LocalInstance`** (chemins forcés sous
  `<pathPrefix>/<namespace>/`, pas de choix de mount/rôle au-delà de la
  release) : le gabarit n'a pas de secret store, rien à brancher dans le
  kit tant qu'aucun opérateur ne construit son `VaultConfig` depuis une
  `LocalInstance`. Exigence écrite sur `SecretStoreFactory`. Les
  `pathGrants` (kit v0.15.0) n'existent que sur l'instance cluster : un
  `VaultConfig` construit pour une `LocalInstance` a `path_grants` vide.
- **Policy par instance** : le kit fournit `governed_by_policy()` ; ce
  sont les reconcilers et webhooks de forgejo/authentik qui doivent
  sauter la policy pour un CR lié à une `LocalInstance`.
- **Refus dès l'admission** de `kind: Instance` / d'un autre namespace
  par un opérateur limité à un namespace, et d'un `sharedWith` non
  accordé : seulement dans les reconcilers (`Ready: False`) — le
  gabarit n'a pas de `/validate`.
- Description du champ `instanceRef.kind` : le schéma généré reprend la
  doc de l'enum (« The kind of instance a reference designates. ») au
  lieu de celle du champ.

**Étape 3 — faite, publiée en v0.10.0** (les opérateurs consomment
depuis le kit par tag, plus par `path`) :

Kit, ajouts pour les opérateurs (non cassants) : `LocalTlsOptions`
(CA dans le namespace de l'instance, `in_namespace(ns)` → `TlsOptions`)
et `LocalSecretKeyRef::in_namespace(ns)`.

Les opérateurs sont branchés sur le kit **en local** (dépendances
`path = "../weebo-operator-kit/crates/…"`, library chart en
`file://../../../../weebo-operator-kit/charts/weebo-operator-lib`) : à
remplacer par `task kit:sync TAG=vX.Y.Z` une fois le kit publié, jamais
poussé tel quel.

*weebo-authentik — fait dans l'arbre* (`task lint`, `task test:all`
verts) :

- `controller(api, &scope)` partout, `Ctx.scope` / `Ctx.instances`,
  watch des `AuthentikNamespacePolicy` seulement avec kinds cluster ;
- `AuthentikLocalInstance` (namespacée : `url`, `tokenSecretRef` et
  `tls` dans son namespace, `sharedWith`) et son controller ; **pas de
  Vault** pour une instance locale : ses applications écrivent des
  `Secret` Kubernetes seulement (une cible Vault échoue avec un message
  clair) — c'est la forme retenue du confinement Vault ;
- `AuthentikApplication.spec.instanceRef` objet (requis), `status.instance`
  (application, et access policy via son application) ; résolution par
  `AuthentikInstances = Instances<AuthentikInstance,
  AuthentikLocalInstance>` ; le cleanup utilise l'instance enregistrée ;
- policy sautée pour un CR lié à une `AuthentikLocalInstance`, dans les
  reconcilers et le webhook (qui résout l'instance ; irrésoluble → la
  policy s'applique) ; `/scope` (garde-fou D4) ;
- codes `InstanceNotAllowed`, `InstanceBindingChanged`,
  `LeavesWatchScope` ;
- binaire : scope, réflecteurs des deux kinds, controllers cluster
  seulement avec kinds cluster, `weebo-authentik-operator
  migrate-instance-ref [--dry-run]` (sur le binaire de l'opérateur : la
  CLI de l'importer n'a pas de sous-commandes) ; importer : `instanceRef:
  {name: …}` ;
- chart : `watch.*`, `webhook.enabled`, params (`authentiklocalinstances`
  dans `kinds`/`namespacedKinds`, pas dans `admitted`) ; rendu full
  namespacé sans objet cluster vérifié ;
- docs : CRD `AuthentikLocalInstance`, exemples `instanceRef` en objet,
  section « A namespace's own instance » (partage, liaison, migration) ;
- test envtest `instance_resolution.rs` : application liée à la
  `AuthentikLocalInstance` → policy sautée + `status.instance` ; liée à
  l'`AuthentikInstance` → `NamespaceNotAllowed`.

Authentik peut tourner en full namespacé : ses kinds namespacés ne
référencent aucun kind cluster (hors instance).

*weebo-forgejo — fait dans l'arbre* (`task lint`, `task test:all` :
372 tests, 0 échec, 39 ignorés = tests contre un vrai Forgejo ;
`task recu` ; rendu full namespacé sans objet cluster) :

- même migration qu'authentik : scope partout (watches des kinds
  cluster User/Organization/Team/NamespacePolicy seulement avec kinds
  cluster), `ForgejoLocalInstance` (Kubernetes seulement, pas de Vault ;
  finalizer « en usage » comme `ForgejoInstance`), `instanceRef` objet
  sur les 9 kinds qui en ont un, `status.instance`, résolution par
  `Instances`, cleanup sur l'instance enregistrée, codes
  `InstanceNotAllowed`/`InstanceBindingChanged`/`LeavesWatchScope`,
  `migrate-instance-ref` sur les 9 kinds, importer, chart, docs ;
- refs : comparaison des instances enregistrées (`default_instance`
  supprimé) ; la recherche des CR encore liés à une instance (son
  finalizer) lit `status.instance`, avec repli sur `instanceRef` pour un
  CR pas encore réconcilié ;
- policy : un CR lié à une `ForgejoLocalInstance` ne garde que la
  vérification de ses `secretTargets` (reconcilers et `/validate`) ;
- tests envtest `instance_resolution.rs` (policy sautée, liaison
  enregistrée, `InstanceBindingChanged` quand on repointe un CR lié) et
  `instance_ref_migration.rs` (User et Repository).

**Écart : règle CEL d'immuabilité de `instanceRef` retirée.** Les CRD de
forgejo portaient une règle de transition « `spec.instanceRef` est
immuable » : l'apiserver l'évalue sur l'ancienne valeur stockée (une
chaîne) et refuse le patch de migration. La règle est retirée ; la même
garantie est tenue par les reconcilers (une instance enregistrée qui ne
correspond plus → `InstanceBindingChanged`, rien n'est touché) : l'édition
est acceptée par l'apiserver, le CR passe `Ready: False`. **À retenir pour
tout opérateur** : une règle de transition sur `instanceRef` bloque
`migrate_instance_refs` ; la retirer dans la release qui migre (elle
pourrait revenir après, une fois tous les CR réécrits).

**Reste pour un forgejo full namespacé** — décision produit à prendre :
ces kinds namespacés référencent des kinds cluster :
`ForgejoRepository` (`organizationRef`, `userRef`, `teamRefs`),
`ForgejoWebhook` (`organizationRef`), `ForgejoAccessToken` (`userRef`,
requis), `ForgejoOAuth2Application` (`ownerRef`, optionnel).
`ForgejoDeployKey` ne référence qu'un dépôt (namespacé). En mode
« un namespace » avec kinds cluster (niveau 2), forgejo fonctionne ; en
full namespacé, ces références ne peuvent pas se résoudre.

### Proposition — forgejo entièrement namespacé (option B, décidée le 2026-10-12 ; D6–D10 validés)

Choix : `ForgejoUser`, `ForgejoOrganization`, `ForgejoTeam` et
`ForgejoUserKey` deviennent namespacés — plus de version cluster. Une
référence vise un CR du même namespace, un CR d'un autre namespace si
c'est activé et accordé, ou un objet Forgejo par son nom (`remoteName`).
`ForgejoInstance` (cluster), `ForgejoLocalInstance` et
`ForgejoNamespacePolicy` (cluster) restent. Release **cassante** de
forgejo (migration hors ligne, ci-dessous).

#### Références

```yaml
organizationRef:
  name: platform            # un ForgejoOrganization ; du namespace du CR par défaut
  namespace: shared-ids     # autre namespace : activé + accordé (D7)
# ou
organizationRef:
  remoteName: platform      # un objet Forgejo existant, non géré par un CR
```

- `name` et `remoteName` exclusifs (règle CEL) ; `namespace` seulement
  avec `name`.
- La cible doit être liée à la même instance (comparaison des
  `status.instance`, déjà en place).
- `remoteName` : l'opérateur vérifie seulement que l'objet existe sur
  l'instance ; il ne le crée, ne le modifie ni ne le supprime jamais.
- Kit (fait) : `weebo-kit-application::sharing::shares_with(shared,
  cr_namespace, labels)`, le consentement générique (la logique de
  `instances::check_shared`, qui l'utilise désormais) ;
  `WatchScope::cross_namespace_refs()` (`WEEBO_CROSS_NAMESPACE_REFS`,
  chart `references.crossNamespace`, toujours faux avec un namespace
  surveillé). Le type de ref reste propre à forgejo (`remoteName` : le
  kit ne parle pas d'objet distant par nom), pas de `CrRef` dans le kit
  tant qu'un seul opérateur en a l'usage.

#### Garde-fous (doublons, double référence)

- Création : Forgejo refuse déjà un nom pris (`ForgejoObjectAlreadyExists`)
  et l'opérateur n'adopte jamais par nom — deux namespaces qui
  déclarent le même compte sur le même serveur : le second échoue.
- **Revendication unique** (D8) : un objet distant (URL du serveur, kind,
  nom Forgejo) ne peut être possédé que par un seul CR, quelle que soit
  son instance (deux instances — cluster et locale — vers le même
  serveur sont légitimes : un namespace avec son propre token). Avant
  toute création ou adoption (importer), l'opérateur cherche un autre CR
  qui le revendique (`status.forgejoId`/`forgejoName` + URL de son
  instance) → `Ready: False` `RemoteObjectClaimed`, rien n'est appelé.
- `remoteName` vers un objet revendiqué par un CR : accepté (lecture
  seule) ; c'est la façon de le référencer sans le partager.

#### NamespacePolicy

Elle gouverne toujours les CR liés à une `ForgejoInstance` (cluster) :
- `allowedKinds` inclut désormais `ForgejoUser`, `ForgejoOrganization`,
  `ForgejoTeam`, `ForgejoUserKey` (un namespace qui crée des comptes sur
  l'instance partagée doit y être autorisé) ;
- `allowedOrganizations` / `allowedUsers` portent sur les **noms
  Forgejo** résolus (et plus sur les noms de CR, devenus ambigus entre
  namespaces) : c'est ce qui empêche un namespace de viser `admin` par
  `remoteName` pour se fabriquer un token (D9).

#### Cross-namespace (D7)

- Activation au niveau de la release : `references.crossNamespace`
  (chart) → `WEEBO_CROSS_NAMESPACE_REFS` ; désactivé par défaut. Sans
  activation, une ref avec `namespace` → `Ready: False`
  `ReferenceNotAllowed`.
- Accord de la cible : `spec.sharedWith` (le type du kit) sur
  `ForgejoUser`/`ForgejoOrganization`/`ForgejoTeam` ; vide = son
  namespace seulement.
- Un opérateur limité à un namespace ne voit pas les autres : la ref
  cross-namespace y est toujours refusée.

#### Migration (cluster → namespacé)

La portée d'une CRD ne change pas en place : il faut supprimer
l'ancienne, ce qui supprime ses CR. Ordre impératif (outil
`migrate-to-namespaced`, `--dry-run` d'abord) :

1. opérateur arrêté (aucun finalizer ne tournera : rien n'est supprimé
   côté Forgejo) ;
2. export de chaque CR cluster (spec + `forgejoId`/`forgejoName`) vers
   un namespace cible : fichier de correspondance (`ForgejoUser/alice:
   team-a`), défaut un namespace donné (`--default-namespace`) ;
3. retrait des finalizers puis suppression des CR, puis des CRD
   cluster ;
4. nouvelles CRD ; recréation des CR dans leur namespace, en pause
   (`import-pending`), status amorcé (l'`Adopter` du kit), relâchés ;
5. réécriture des refs des CR namespacés existants (`ForgejoRepository`,
   tokens, webhooks…) : `namespace` ajouté quand la cible a changé de
   namespace, et `sharedWith` amorcé sur la cible pour ces namespaces ;
   si des refs cross-namespace en résultent, l'outil exige
   `references.crossNamespace` ;
6. opérateur démarré.

Les manifests GitOps doivent être réécrits dans la même release (l'outil
écrit les YAML). Les objets Forgejo ne sont jamais touchés.

#### Décisions (validées le 2026-10-12, telles que recommandées)

- **D6** — `ForgejoUserKey` suit `ForgejoUser` (namespacé, même namespace
  que son user ou cross-namespace accordé). Recommandé : oui.
- **D7** — cross-namespace = activation release **et** accord de la
  cible (`sharedWith`). Recommandé : les deux.
- **D8** — revendication unique d'un objet distant par CR, toutes
  instances confondues (par URL de serveur). Recommandé : oui.
- **D9** — dimensions de la policy sur les noms Forgejo. Recommandé :
  oui (sinon `remoteName` contourne la policy).
- **D10** — migration par l'outil ci-dessus (namespace cible par
  correspondance, défaut configurable). Recommandé : oui.

#### État — fait, publié dans weebo-forgejo v0.2.0 (kit v0.10.1)

`task lint`, `task test:all` (394 réussis, 0 échec, 39 ignorés = tests
contre un vrai Forgejo), `task recu`, `task lint:helm`, rendu full
namespacé sans objet cluster.

- D6 : les 4 kinds namespacés ; `sharedWith` sur User/Organization/Team ;
  chart (`kinds`, `namespacedKinds`, `admitted`, `references.crossNamespace`).
- Refs : `IdentityRef { name?, namespace?, remoteName? }` (CEL : un seul
  de `name`/`remoteName`, `namespace` seulement avec `name`) ; refus
  `ReferenceNotAllowed` sans activation ou sans accord ; `remoteName` ne
  vérifie que l'existence (un `remoteName` d'équipe n'est pas vérifié :
  Forgejo valide).
- D8 : revendication (URL du serveur, kind, nom Forgejo ; équipes en
  `org/équipe`) avant création d'un user/organisation/équipe et à
  l'adoption de l'importer → `RemoteObjectClaimed`. Limites : users en
  mode Observe exclus ; en full namespacé l'opérateur ne voit que les
  revendications de son namespace ; l'importer saute le contrôle quand
  aucune instance n'est encore résoluble.
- D9 : `allowedKinds` couvre les 4 kinds ; `allowedUsers` /
  `allowedOrganizations` sur les noms Forgejo.
- D10 : `weebo-forgejo migrate-to-namespaced` (`--dry-run`, `--yes`,
  `--out`, `--mapping`, `--default-namespace`,
  `--allow-cross-namespace`), plan écrit dans `out/plan.json` et
  reprise possible ; réécrit les refs et l'annotation propriétaire des
  `Secret` Kubernetes. Le binaire de l'opérateur dépend désormais du
  crate importer (la logique y vit).
- Importer : CR écrits dans `--namespace`.
- Tests envtest : `identity_refs.rs`, `full_namespaced.rs` (un seul
  `Role`), `importer/tests/migrate_namespaced.rs`.

Reste ouvert :
- ~~Propriétaire des valeurs Vault non renommé~~ réglé : le kit accepte
  un ancien identifiant de propriétaire (`SecretOwner::formerly`, lu dans
  l'annotation `<domaine>/previous-owner` — `Annotations::previous_owner`)
  et réécrit la valeur sous le nouveau ; la migration pose l'annotation
  sur chaque `ForgejoUser`/`ForgejoUserKey` déplacé.
- `secretTargets` d'un user vers un autre namespace : la migration ne
  fait qu'avertir.
- Cassant pour les installations : ajouter les 4 kinds aux
  `allowedKinds` des policies existantes, et passer par la migration.

### CD commune — `publish.yml` réutilisable (2026-10-13)

Constat : authentik publiait son image sans la scanner (même
`libssl3` High CVE-2026-84782 que forgejo, dont le scan Trivy bloquait
v0.2.0) ; le scan Trivy du variant Red Hat Hummingbird de forgejo était
aveugle (`ID=hummingbird` : OS non supporté, rien n'est lu).

- `.github/workflows/publish.yml` (`workflow_call`), tiré de celui de
  forgejo : contrôle de version (Chart/appVersion/Cargo = tag), une image
  par variant (`variants` : Containerfile, suffixe, `required`),
  multi-arch, SBOM + provenance, **scan Grype** avant et après le push
  (`--fail-on high` : toute High/Critical bloque, corrigeable ou non —
  décidé ; exceptions dans le `.grype.yaml` de l'opérateur), smoke-run
  (`smoke-entrypoint`/`smoke-args`), signature cosign ; chart OCI signé ;
  release GitHub (notes cog, bundle CRD signé). Un variant optionnel en
  échec est laissé de côté (le chart et la release partent), un variant
  requis arrête tout. La porte e2e reste dans l'opérateur (un job dont
  l'appel dépend).
- Images de base (décidé le 2026-10-13, après un scan Grype à base à
  jour) : aucune image avec libc n'est sans High — CVE-2026-95619 touche
  le runtime GCC (`libgcc`/`libstdc++`) de Hummingbird comme de Debian 13
  (« won't fix » chez Debian), qui a aussi `libc6` et `zlib1g`. Retenu :
  - **image principale = Red Hat Hummingbird** (`hi/core-runtime`,
    builder `hi/rust`), CVE-2026-95619 **acceptée** dans le `.grype.yaml`
    de l'opérateur (noms de paquets en regex : `^libgcc$`,
    `^libstdc\+\+$`), à retirer au correctif ;
  - **variant `-static`** (optionnel) : binaire musl statique
    (`rust:1-slim-trixie` + `musl-tools`, `ring`/`aws-lc-sys` compilent)
    sur `hi/static` — aucune faille ; même UID 65532, mêmes chemins de CA
    (`/etc/ssl/certs/ca-bundle.crt`). Prototype : 39 Mo, `--version` ok.
  - Debian distroless abandonné.
- Gabarit : `Containerfile` (Hummingbird), `Containerfile.static`,
  `.grype.yaml`, `.github/workflows/publish.yml` (chart et image déduits
  du nom du dépôt), `--version` du binaire pour le smoke-run, lint des
  deux Containerfiles.

- Premier passage réel (authentik v0.16.1, forgejo v0.2.1) : images
  construites, poussées et scannées propres, mais le smoke-run arm64
  échouait (`docker: cannot overwrite digest` — avec le store d'images
  containerd, re-tirer le digest d'index pour une seconde plateforme est
  refusé) ; rien n'a été signé, ni chart ni release. Corrigé en v0.12.1 :
  chaque plateforme tourne sur le digest de son propre manifeste. Les
  images v0.16.1/v0.2.1 restent dans GHCR, non signées ; les releases
  utilisables sont les patchs suivants.
- arm64 hors du chemin critique (décidé le 2026-10-13 : pas d'ARM en
  production pour l'instant ; le build arm64 sous QEMU coûtait ~1h25 par
  variant à chaque release). v0.13.0 : `platforms` vaut `linux/amd64` par
  défaut (QEMU n'est installé que si on demande autre chose) ; `arm64:
  true` ajoute un job `image-arm64` par variant, sur runner arm64 natif
  (`ubuntu-24.04-arm`, gratuit pour les dépôts publics), publié sous son
  propre tag `vX.Y.Z[suffixe]-arm64` (+ `latest-arm64`/`static-arm64`),
  scanné, smoke-runné et signé, jamais bloquant (la release le liste s'il
  est sorti). Le jour où ARM devient une cible : soit `arm64: true`, soit
  un index multi-arch unique (`platforms: linux/amd64,linux/arm64`, sous
  QEMU, bloquant) — à terme, assembler l'index à partir des deux builds
  natifs (`imagetools create`) serait le mieux des deux.

### v0.14.0 — corrections de la revue du 2026-10-13 (cassante)

Détail des points : `docs/review-2026-10-13.md` (K1–K18).

Comportement (sans changement d'API) :

- **Instance par défaut stable** : sans `instanceRef`, l'instance
  enregistrée dans le status reste tant qu'elle est encore candidate
  (`instances::resolve_bound`, utilisé par `Instances`). Une instance
  créée après coup (une `Instance` cluster, une seconde locale) ne prend
  plus l'objet en silence ; l'instance enregistrée disparue et une autre
  disponible → `ResolveError::BindingChanged` (message reformulé).
- **Vault** : un 403 ne relance un login que si `lookup-self` échoue
  aussi (jeton réellement périmé) — un refus de policy ne crée plus de
  jeton ; le JWT du ServiceAccount est relu à chaque re-login
  (`VaultSecretStore::with_jwt_path`, posé par `VaultLogins`). Suppression
  sans droit `delete` sur `<mount>/metadata/*` : repli sur la suppression
  de la version courante (`<mount>/data/*`), les anciennes versions
  restent (warning) — le droit sur `metadata` reste recommandé.
- `K8sSecretStore` réécrit l'annotation de propriétaire tant qu'elle
  porte l'ancien id ; `SecretOwner::formerly` n'est accepté que pour le
  même kind et le même nom (seul le scope change).
- `drive` ne remet plus la watch « en bonne santé » sur un reconcile
  réussi (la série d'échecs se termine après `FAILURE_STREAK_GAP`).
- Requeue jamais sous `MIN_REQUEUE_SECS` (1 s) ; reflectors attendus au
  plus `STORE_READY_TIMEOUT` (10 s), puis lecture par l'API.
- Notes d'Event tronquées en octets (1000) sur une frontière de
  caractère ; advisories d'une même raison fusionnées en une condition ;
  patch de status non rejoué sur une erreur de (dé)sérialisation.
- Doc : `SharedWith.namespaceSelector` vide = tout le cluster ; doc du
  lease (le raccourci « stale » lisait l'horloge murale : retiré en
  v0.14.1) ; plus aucun nom de
  produit ni champ de CRD d'opérateur dans les doc comments du kit (les
  CRD générées changent : `task recu` côté opérateur).

Ajouts :

- `annotations::INSTANCE_TOKEN` (`<domaine>/instance-token`, un
  **label** de Secret), `Annotations::instance_token_allowed`, et
  `weebo_kit_adapters::read_local_instance_token` (+ `TokenSecretError`) :
  le jeton d'une instance **locale** n'est lu que si son Secret porte
  `<domaine>/instance-token: "true"`. Sinon, quiconque peut créer une
  instance locale ferait envoyer n'importe quel Secret du namespace vers
  une URL de son choix (l'opérateur lit avec ses propres droits). Les
  instances cluster gardent `read_secret_key`.

**Cassant pour les opérateurs** (à faire au bump) :

1. `RefState::NotReconciled` / `RefError::NotReconciled` (variantes
   ajoutées) : un CR référencé sans condition `Ready` n'est plus
   `NotReady("NotReconciledYet")`. Ajouter les bras aux `match`
   exhaustifs (forgejo : `use_cases/refs.rs`, `require` → son code
   `ReferenceNotReady`, `resolve_member` → `Member::Pending` avec un
   message propre) et mettre à jour les tests qui attendaient la chaîne.
2. Instances locales : lire le jeton avec `read_local_instance_token`
   (authentik : `gateway_factory.rs`, forgejo : l'équivalent), traduire
   `TokenSecretError::NotAllowed` en code de raison, documenter le label
   (page « connecter une instance »), et **prévenir les utilisateurs** :
   les Secrets existants doivent être labellisés avant la mise à jour.
3. CRD régénérées (`task recu`) : descriptions de `secretTargets[].backend`,
   `tls`, `sharedWith.namespaceSelector` modifiées.
4. Tests d'opérateur qui comptaient sur un re-login à chaque 403 Vault
   (scriptés sans `lookup-self`) : ajouter la réponse `lookup-self`.

### v0.14.1 — prise du lease sur l'horloge monotone seule

`LeaseLock` ne prend plus un lease dont le `renewTime` paraît vieux de
plus de 3 TTL selon l'horloge murale locale : un nœud en avance de plus
de 45 s pouvait déposer un leader vivant. Comme client-go, un candidat ne
prend le lease qu'après l'avoir vu inchangé (même `resourceVersion`)
pendant `leaseDurationSeconds` sur sa propre horloge monotone ; le
décalage entre horloges ne compte plus. Contrepartie : à côté d'un leader
mort sans avoir rendu le lease, la reprise attend un TTL (15 s). Une
release qui s'arrête proprement le rend (`Leadership::release`).
`leader::STALE_FACTOR` est déprécié (plus utilisé). Rien à changer côté
opérateurs.

### v0.14.2 — le template lit le jeton d'une LocalInstance avec l'opt-in

`templates/operator` lisait le jeton d'une `<Product>LocalInstance` avec
`read_secret_key` : un opérateur généré contournait le label
`<domain>/instance-token: "true"` de la v0.14.0. La gateway factory du
template passe désormais par `read_local_instance_token` ; un Secret sans
label donne `TokenSecretNotAllowed` (nouveau code du template). Les
opérateurs existants (forgejo, authentik) le faisaient déjà ; rien à
changer pour eux.

### v0.14.3 — licence du template en placeholder

`templates/operator` écrivait `license = "MIT"` et
`org.opencontainers.image.licenses="MIT"` en dur, alors que les opérateurs
n'ont pas tous la même licence (forgejo et batlehub : Apache-2.0 ;
authentik : MIT). Nouveau placeholder `license` (`Apache-2.0` par défaut,
ou `MIT`), utilisé par `Cargo.toml` et les deux Containerfiles. Le fichier
de licence reste à ajouter par l'opérateur. Rien à changer pour les
opérateurs existants.

### v0.15.2 — corrections de la revue du 2026-10-15 (non cassante)

Comportement :

- **Propriété Vault sans course** : avec les `pathGrants`, plusieurs CR
  visent le même chemin par construction ; lire l'owner, écrire, puis
  enregistrer l'owner en trois appels laissait deux reconciles
  concurrents finir avec la valeur de A et B comme owner.
  `RoutingSecretStore` écrit désormais ainsi : chemin d'un autre → refus ;
  chemin libre ou sans owner (adopté, comme avant) → l'owner est
  enregistré d'abord puis relu (de deux revendications, celle relue gagne,
  l'autre est refusée) ; la valeur est écrite en **check-and-set** KV v2
  sur la version lue avec l'owner (`cas=0` à la création : un seul
  créateur gagne) ; le perdant relit l'owner et reçoit le refus habituel
  (« belongs to … »), ou une erreur de conflit rejouée au reconcile suivant
  si l'owner n'a pas changé ; après l'écriture, l'owner est relu une
  dernière fois (revendication reprise entre-temps → refus). Sans droit
  `read` sur `<mount>/metadata/*`, rien ne change (écriture sans CAS,
  propriété non vérifiée). Ajouts : `VaultSecretStore::metadata`
  (`PathMetadata`), `write_path_cas` (`CasWrite`). Vérifié contre OpenBao
  (métadonnée seule = version 0, `cas=0` accepté puis refusé).
- **Admission = reconcile** : `admission::check_vault_targets` applique
  `emit::check_target` (cible Vault d'un objet namespacé nommant un autre
  namespace → `CrossNamespace` ; objet cluster sans namespace ni chemin →
  `MissingNamespace`), et le sous-arbre d'un objet namespacé est celui de
  son namespace, et non celui que la cible nomme. `weebo-kit-runtime`
  dépend désormais de `weebo-kit-application` (aucun client ajouté).
- Doc : un `pathPrefix` vide fait de chaque namespace un dossier de
  premier niveau du mount (un namespace nommé comme un dossier ouvert par
  un grant le possède) : avertissement sur `VaultConfig::path_prefix`,
  `vault_paths::subtree` et dans la RFC §2. À reprendre dans la
  description du champ `pathPrefix` de chaque opérateur.
- Plus aucun nom de produit dans le code et les commentaires du kit
  (exemple de `VaultPathGrant.paths` : `shared/auth` — description de CRD
  modifiée, `task recu` côté opérateur ; tests ; `weebo-kit-perf` ;
  `operator-docs`, dont le nom du thème Shiki `weebo-css-variables`).

Ajouts (aucun opérateur ne casse) :

- `controller::finalize_with(obj, recorder, annotations, released,
  cleanup, always)` : `always` tourne à chaque suppression, force-orphan
  compris (ce que l'opérateur a accordé *sur le cluster* à la CR : un
  `ClusterRoleBinding`…). `finalize` = `finalize_with` sans hook.
- `status::record(api, field_manager, name, &fields)` : un merge patch de
  champs de status, rejoué comme `publish` sur une erreur transitoire,
  jamais sur une erreur de (dé)sérialisation — pour enregistrer ce qui
  vient d'être écrit à distance sans attendre le `publish` final.
- `store::spawn_reflector(api, scope, health, name)` et
  `leader::register_leader_gauge(meter, prefix)` (+ `_on(&Meter, prefix)`)
  : les copies des `main.rs` des opérateurs (le template les utilise).
  Le générateur de référence de `operator-docs` reconnaît
  `register_leader_gauge(` et `finalize_with(`.
- `pod::own_labels()` / `pod::own_namespace()` et, dans le chart,
  `WEEBO_OPERATOR_POD_LABELS` (labels de sélection du pod, JSON,
  `nameOverride` appliqué) : de quoi écrire une NetworkPolicy qui laisse
  entrer l'opérateur.
- Chart : `params.watchRules`, règles ajoutées au Role du namespace
  surveillé (`watch.namespace`) — un release `watch.clusterKinds: false`
  ne rend pas de ClusterRole, donc aucune `clusterRules`. Le Role est
  rendu dès qu'il y a des `namespacedKinds` ou des `watchRules`.
  `task chart:check` le vérifie (`charts/testdata/watch-rules.yaml`),
  `helm:lint` vérifie la variable d'environnement.

Côté opérateurs (à l'adoption) : remplacer `spawn_reflector` et
`register_leader_gauge` locaux, passer les règles namespacées dans
`watchRules` (batlehub : spec §7.1), `record` pour les champs de status
écrits après un effet distant, `task recu` (description de
`pathGrants[].paths`), nouvelle somme `sha256` de `tasks/helm.yaml`.

### v0.17.0 — le socle « workload géré » (extrait de weebo-batlehub)

Le quatrième opérateur (weebo-angos) déploie, comme batlehub en mode
`managed`, le logiciel qu'il pilote. Ce que batlehub portait seul
(`application::workload`, `pod_safety`, `adapters-outbound::workloads`)
entre dans le kit, neutre (règle des deux opérateurs : batlehub l'adopte
dans la même release, angos le prend au M0).

`weebo-kit-application` (pur, types `k8s-openapi` seulement ; nouvelles
dépendances `serde`, `serde_json`, `sha2`) :

- `workload` : `OwnedObject` (+ `OwnedObject::of(&k8s_obj, create_only)`),
  `Owner`, `Rollout` (annotations du template au lieu d'un `config_hash`
  propre à batlehub : `rollout.annotation(KEY)`), `WorkloadError` neutre
  (`Conflict`, `Kubernetes`, `Refused`, `SecretNotAllowed` : l'opérateur
  le traduit en ses codes), `Listed`, `PodState`.
- Ports `OwnedObjects` (`apply`, `remove`, `list`, `secret_text`,
  `rollout`, `annotate_template` — redémarrage compris —, `pods`,
  `claim_size`) et `PodLogs` (`read(ns, pod, LogQuery{container, since,
  limit_bytes})`, lignes horodatées par le kubelet, `parse_timestamped`).
- Objets nommés par hash : `content_hash`, `hashed_name(base, content)`
  (`<base>-<8 hex>`), `stale(listed, in_use, keep)` (ce qu'on peut
  supprimer en gardant les `keep` plus récents pour un `rollout undo`).
- `OperatorPod` (`named(chart)`, `peer()`), `network_policy(meta,
  &PolicyRules)` (l'opérateur n'entre que si `operator` est donné et que
  `ingress_from` n'est pas vide), `restricted_pod_security(uid)`,
  `restricted_container_security()`.
- `pod_safety` : allow-list d'images (`parse_allowed_images(setting,
  defaults)`, `repository`), `violations`, `secret_names`,
  `pod_templates` (Deployment, StatefulSet, DaemonSet, ReplicaSet, Job,
  CronJob, Pod), `problems`, `summary`, `secrets_to_check`,
  `secret_not_labelled(ns, name, label)`. Plus aucun texte ni code propre à
  un produit : l'opérateur formule le refus.
- `test_support` : `FakeOwnedObjects`, `FakePodLogs`.

`weebo-kit-adapters` : `KubeOwnedObjects::new(client, WorkloadSettings
{field_manager, managed_by, owner_api_version, owner_kind,
credential_label, allowed_images})` (SSA forcé, aucune reprise implicite,
pods vérifiés avant toute écriture), `is_ours(meta, owner, managed_by)`,
`kube_error`, `pod_state(&Pod)`, `KubePodLogs` (`pods/log` en `get`).
Test de contrat `workloads_contract` (wiremock).

`weebo-kit-runtime` : `pod::operator_pod(chart)` (labels du chart ou
repli sur `app.kubernetes.io/name=<chart>` avec un avertissement, et
`POD_NAMESPACE`).

Rien ne casse pour forgejo et authentik (ajouts seulement). Côté batlehub
(adopté) : `ports` réexporte les types du kit et traduit par
`workload_reason(&e)` ; `Workloads: OwnedObjects` ne garde que le jeton
de l'opérateur, la liaison TokenReview et `restart` (par défaut, sur
`annotate_template`) ; `KubeWorkloads` délègue à `KubeOwnedObjects` ;
`FakeWorkloads` enveloppe `FakeOwnedObjects` ; `OperatorPod::default()` →
`OperatorPod::named(OPERATOR_CHART)`.
