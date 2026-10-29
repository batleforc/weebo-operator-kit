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
| `weebo-kit-domain`      | domain       | rien                          | `Reason`/`Severity`, `reason_codes!`, `Failure<R>`/`Advisory<R>`, moteur d'allow-list générique, comparaison avant écriture (`drift` : `Drift`, `same_set`…, `RemoteWrite`) |
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
  `WEEBO_RECONCILE_CONCURRENCY`, 4 par défaut), `runtime::build` (runtime
  tokio à `WEEBO_WORKER_THREADS` workers, 4 par défaut, pas un par cœur),
  `drive`, `jittered`,
  `requeue_after(obj, steady, due_in)` (300 s/30 s ±20 %, le délai
  d'échec doublé à chaque retry du CR qui échoue encore jusqu'à 300 s ;
  un échec vu plus tôt, sur un événement de watch, ne compte pas),
  `retry_after_error` (corps de l'`error_policy` : Event `Warning` +
  même backoff), `finalize` (cleanup sauf `force-orphan`, puis Event du code
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
- Versions alignées sur forgejo : fumadocs-core/ui ^16.16.2, fumadocs-mdx
  ^15.4.6, next ^16.4.1, react 19.3, en `peerDependencies` par plages (un
  correctif de sécurité de Next n'attend pas une release du kit) ; les
  `devDependencies` épinglent les planchers.
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

`LocalInstances<L>` : l'`InstanceDirectory` d'un opérateur sans kind
d'instance cluster (le `LocalInstanceDirectory` de batlehub, rendu
générique) — `resolve` ne partage avec personne, `resolve_with(…,
sharing)` prend la règle de partage de l'opérateur (le
`registrySelector` de batlehub, le `repositorySelector` d'angos) ;
`instances(ns)`, `get(bound)`, `with_store`, `with_scope`. Contrat dans
`instances_contract`.

`weebo-kit-api` : `AttachNamespaces {from, selector}` et `NamespacesFrom`
(`Same`/`All`/`Selector`, règle CEL « selector requis avec Selector ») —
le consentement d'une instance aux objets d'autres namespaces, à la
manière de `allowedRoutes.namespaces` de la Gateway API ; `shared_with()`
le traduit en `SharedWith` pour `LocalInstances::resolve_with`. Ajout seul
(le `RegistryNamespaces` de batlehub en devient un alias, CRD inchangés).

`weebo-kit-runtime` : `pod::operator_pod(chart)` (labels du chart ou
repli sur `app.kubernetes.io/name=<chart>` avec un avertissement, et
`POD_NAMESPACE`).

Rien ne casse pour forgejo et authentik (ajouts seulement). Côté batlehub
(adopté) : `ports` réexporte les types du kit et traduit par
`workload_reason(&e)` ; `Workloads: OwnedObjects` ne garde que le jeton
de l'opérateur, la liaison TokenReview et `restart` (par défaut, sur
`annotate_template`) ; `KubeWorkloads` délègue à `KubeOwnedObjects` ;
`FakeWorkloads` enveloppe `FakeOwnedObjects` ; `OperatorPod::default()` →
`OperatorPod::named(OPERATOR_CHART)` ; `LocalInstanceDirectory` enveloppe
`LocalInstances`.

### Lectures des stores sans attente de 10 s (correctif, non cassant)

`Store::wait_until_ready` de kube ne réveille que la dernière tâche qui
l'a interrogé (un seul oneshot) : quand plusieurs reconciles attendaient
le même store pas encore listé, un seul était réveillé, les autres
attendaient les 10 s de `STORE_READY_TIMEOUT` puis listaient l'API.
`weebo_kit_adapters::wait_synced(&store, timeout) -> bool` (repris de
weebo-forgejo) revérifie toutes les 50 ms (`STORE_POLL`) ; `Instances`,
`LocalInstances` et `InstanceResolver` s'en servent. Dans
`weebo-kit-adapters` et non dans le runtime : un adaptateur sortant ne
dépend pas du runtime.

### Une watch par kind, partagée par tous les contrôleurs (ajout, non cassant)

Repris de weebo-forgejo (`controller/shared.rs`), rendu générique :
chaque `Controller::watches` ouvrait sa propre watch, et toutes listaient
au démarrage des contrôleurs, juste après la prise du lease.

`weebo_kit_runtime::shared` :

- `SharedWatch<K>` : un reflector dont les changements sont diffusés aux
  abonnés (`touched()` pour une watch de kind référencé, `controller()`
  pour le kind principal — mêmes déclencheurs et même concurrence que
  `controller::controller`) ; un abonné reçoit tout le store une fois listé,
  puis chaque changement, et relit tout le store s'il a pris du retard.
- `SharedWatches` : les watches du binaire, une par kind et sélecteur de
  labels (indexées par type, aucun champ nommé) — `spawn(api, &scope,
  &health, name) -> Store<K>`, `get::<K>()`, `get_in::<K>(&scope)`,
  `touched::<K>(&scope, &client)`, `controller(api, &scope)` (la watch partagée, sinon le
  contrôleur du kit sur sa propre watch), `listed()`,
  `wait_listed(LISTED_TIMEOUT)`.
- `WatchesIn::watches_in(&watches, &scope, &client, map)` (et
  `watches_in_filtered(…, predicate, map)`) : la watch d'un kind
  référencé, partagée si possible, ignorée pour un kind cluster quand
  l'opérateur n'en surveille aucun.
- `LISTED_TIMEOUT` (30 s) : le réplica ne fait campagne pour le lease
  qu'une fois ses watches listées, au plus 30 s.

Le template : `controller::spawn_watches` (instances, instances locales,
objets) ; les contrôleurs passent par `ctx.watches.controller(…)`, les
stores de `Instances` sont ceux des watches partagées, `main` attend
`wait_listed` avant `acquire_leadership`. `controller::controller` reste
tel quel : un opérateur l'adopte quand il veut (`Ctx.watches` est un
champ de plus côté template, pas dans le kit).

Publié en v0.20.0. Reste à faire côté opérateurs (demandé le 2026-10-19) :
passer weebo-authentik, weebo-batlehub, weebo-angos et weebo-stalwart sur
`SharedWatches` (contrôleurs par `watches.controller(…)`, kinds référencés
par `watches_in`, campagne pour le lease après `wait_listed`), et
weebo-forgejo de sa copie locale (`controller/shared.rs`, sa PR #16) à
celle du kit. C'est un changement de code par opérateur, après le
`kit:sync` v0.20.0 ; `task perf` le valide (requêtes/min au repos sous
`Budget::small_cluster()` avec 4 workers : forgejo est passé de 37 watches
à 15). Le correctif `wait_synced` arrive, lui, avec le seul `kit:sync`.

Fait le 2026-10-19 dans les cinq opérateurs (forgejo 15 watches, authentik
15 → 12, batlehub 16 → 9, angos 12 → 7, stalwart 15 → 8), budget tenu. Trois
manques de l'API relevés en route, corrigés dans le kit (ajouts, non
cassants : aucun opérateur ne casse à la compilation ni ne change de
comportement tant qu'il ne s'en sert pas) :

1. **Une seule config de watch par kind** — corrigé. `SharedWatches`
   indexe par (type, sélecteur de labels) : `spawn(api, &scope, …)` ouvre
   la watch du kind à travers le sélecteur de `scope`, sauf si elle
   existe ; à travers `scope.without_selector()`, celle de tous les objets
   (la même watch quand l'opérateur n'a pas de sélecteur). `get_in::<K>
   (&scope)` rend la watch d'un sélecteur ; `get::<K>()` reste « la
   première ouverte ». `controller(api, &scope)` ne prend que la watch du
   sélecteur de `scope` (jamais une watch plus large : il réconcilierait
   hors du scope), sinon sa propre watch ; `watches_in` prend celle du
   sélecteur, sinon celle de tous les objets (plus d'objets, jamais
   moins), sinon sa propre watch — le comportement d'avant pour forgejo,
   qui lit ses policies sans sélecteur à travers `ctx.scope`. authentik
   partage alors les deux watches de `AuthentikNamespacePolicy` (sans
   sélecteur pour les policies, avec pour son contrôleur), attendues
   toutes deux par `wait_listed`.
2. **Pas de `WatchScope::without_selector()`** — corrigé.
   `scope.without_selector()` garde le namespace, le drapeau des kinds
   cluster et les références inter-namespaces : plus besoin de revérifier
   `cluster_kinds()` avant `watches_in`.
3. **`watches_in` ne prend pas de filtre de changement** — corrigé.
   `WatchesIn::watches_in_filtered(&watches, &scope, &client, predicate,
   map)` (un `Predicate` de kube, p. ex. le hash du champ de statut rendu)
   et `SharedWatches::touched::<K>(&scope, &client)` (le flux que
   `watches_stream` prend, `None` pour un kind cluster quand l'opérateur
   n'en surveille aucun) ; même choix de watch que `watches_in`.

Ce que chaque opérateur peut alors retirer, au `kit:sync` suivant :
weebo-forgejo et weebo-authentik leur `controller::unselected`
(`scope.without_selector()`), authentik aussi la revérification de
`cluster_kinds()` de `on_policy_change` et la watch propre du contrôleur de
`AuthentikNamespacePolicy` (`spawn` à travers `ctx.scope` en plus de celle
sans sélecteur, puis `ctx.watches.controller(…)`, et `get_in` au lieu de
`get` pour le store des policies) ; forgejo peut de même partager la watch
de son contrôleur de `ForgejoNamespacePolicy` et passer ses reflectors
d'instances sans sélecteur sur `spawn(…, &scope.without_selector(), …)` ;
weebo-angos son `controller::touched` (`watches_in_filtered`).

### À faire — une cause `spec` pour les écritures distantes

`RemoteWrite` n'a que `Created`, `Drift` et `Deleted` : une écriture qui
applique un changement de spec est comptée `cause="drift"` dans
`<prefix>_remote_writes_total`, comme une vraie dérive corrigée. L'Event
`DriftCorrected` fait déjà la différence (rien quand `metadata.generation !=
status.observedGeneration`), pas la métrique. Conséquence, relevée sur
weebo-batlehub en comptant ses écritures (2026-10-19) : une série de
modifications de spec en moins de 30 minutes déclenche `RepeatedDrift`,
alors qu'aucun objet n'a dérivé.

Piste : `RemoteWrite::Spec` (`cause="spec"`), choisi par l'appelant avec la
même règle que l'Event (génération pas encore observée), et
`RepeatedDrift` qui ne compte que `cause="drift"` (déjà le cas de son
sélecteur). Ajout non cassant pour la métrique ; les opérateurs qui
choisissent la cause eux-mêmes (`record_write`) l'adoptent un par un, la
page de métriques d'`@weebo/operator-docs` liste la nouvelle valeur.

### À faire — l'IDE à CRD (décidé le 2026-10-21)

Le formulaire de manifeste (`CrdBuilder`/`CrdForm`) édite une ressource
d'un kind à la fois. L'IDE à CRD est un outil distinct, à côté : il
compose un **projet** de ressources liées, d'un ou de plusieurs
opérateurs, et le montre en formulaire et en graphe. Il reste dans le
modèle du site : 100 % statique, aucun accès à un cluster.

#### Périmètre

- Trois publics : l'utilisateur qui découvre un opérateur (mode guidé, par
  recettes), l'admin qui maintient des manifestes GitOps (import, édition,
  export), le développeur (démo, vérification d'un assemblage).
- Un seul composant, deux usages : sur le site d'un opérateur, ses CRD ;
  sur le site du kit (`site/`), les CRD de tous les opérateurs weebo plus
  un catalogue de CRD tierces (External Secrets, cert-manager…).
- Validation côté client : schéma structurel (types, requis, enum,
  pattern, bornes) et résolution des liens (cible absente du projet,
  mauvais kind, référence inter-namespace sans `sharedWith`). Les règles
  CEL (`x-kubernetes-validations`) sont affichées, pas évaluées :
  l'API server le fera.
- Hors périmètre : l'interconnexion des opérateurs **au runtime** (un CR
  d'un opérateur consommé par un autre dans le cluster, types de
  référence inter-opérateurs dans `weebo-kit-api`). Sujet à part, à
  instruire avant toute ligne de code.

#### Les liens

Le schéma OpenAPI ne dit pas qu'un champ pointe vers un kind. Kubernetes
refuse les extensions `x-…` arbitraires dans `openAPIV3Schema` : l'info ne
peut pas rester dans la CRD.

- **Opérateurs weebo** : un marqueur `x-weebo-link` posé sur le champ
  (`#[schemars(extend(…))]`), par défaut sur les types du kit qui sont des
  références (`InstanceRef`, `SecretKeyRef`, `LocalSecretKeyRef`) ; pour un
  `ObjectRef`, l'opérateur ajoute le kind cible sur le champ. La CRD ne
  le garde pas d'elle-même : `CustomResourceExt::crd()` passe le schéma par
  `JSONSchemaProps`, qui ignore les clés inconnues (un test du kit le
  vérifie). `weebo_kit_api::links::of::<K>(&roles)` génère le schéma à
  nouveau, avec les réglages de kube, et lit les marqueurs ; les rôles
  `Instance`/`LocalInstance` d'`InstanceRef` deviennent les kinds de
  l'opérateur. `crdgen` (template) écrit `deploy/crd/links.json`
  (`{ "version": 1, "kinds": { <Kind>: [{ path, to: [{ group, kind }] }] } }`,
  chemins triés, `[]` pour les items d'un tableau, `*` pour les valeurs
  d'une map). `helm:crds` et `weebo-crd-docs` ne lisent que les `*.yaml` :
  le fichier ne les gêne pas. `weebo-crd-docs` publie `links.json` à
  côté de `crd-schemas/`. `weebo-kit-api` gagne `serde_json`,
  déjà une dépendance de schemars.
- **CRD tierces** : une table `CrdLinks`, donnée de la config de docs (comme
  `CrdChoices`), jamais dans le paquet.
- Deux sortes de liens : la **référence** (un champ nomme une ressource du
  projet : `instanceRef` → une instance) et la **production** (une
  ressource produit un objet Kubernetes : un `ExternalSecret` produit le
  Secret de `spec.target.name`, qu'un `SecretKeyRef` consomme). Les objets
  produits apparaissent comme nœuds dérivés ; un Secret créé hors du
  projet est un nœud externe déclaré à la main (nom, clés), jamais
  exporté. La production se déclare dans `CrdLinks`.

#### Sources des schémas

- Site d'un opérateur : ses `crd-schemas/` et son `links.json`.
- Site du kit : au build, les schémas d'une release taguée de chaque
  opérateur weebo (liste et tags épinglés dans `site/`), plus un catalogue
  de CRD tierces à versions épinglées (`site/`, mis à jour comme une
  dépendance, un test vérifie qu'elles se parsent). Le paquet ne nomme
  aucun produit.
- Partout : glisser-déposer d'un CRD YAML, chargé localement.
- CRD à plusieurs versions : l'IDE propose la version de stockage ; un
  manifeste d'une autre version servie s'importe en lecture seule, avec un
  avertissement. Pas de conversion.

#### Données

- **Le projet, c'est le YAML** : pas de format maison. Import d'un YAML
  multi-documents (les liens sont reconstruits par résolution des
  références), export en YAML multi-documents trié par dépendances
  (Secrets et instances d'abord) ou en zip d'un fichier par ressource.
  Kustomize plus tard.
- Brouillon dans `localStorage` (lecture et écriture protégées, la page
  marche sans) ; les positions du graphe sont recalculées par auto-layout.
- Namespace par défaut du projet, surchargeable par ressource ; les kinds
  cluster-scoped n'en ont pas.
- **Recettes** (mode guidé) : un YAML multi-documents avec des trous
  (`<à remplir>`), importé par le même chemin. Celles d'un opérateur dans
  sa config de docs ; celles qui croisent les opérateurs dans `site/` du
  kit.

#### Technique

- Sous-chemin `@weebo/operator-docs/crd-ide`, chargé à la demande
  (`next/dynamic`) sur sa seule page : les autres pages ne paient pas le
  graphe. Nouvelle dépendance `@xyflow/react` : release. L'auto-layout est
  dans le modèle, sans bibliothèque.
- Un **modèle TS pur**, sans React : graphe, résolution des liens,
  validation, import/export, recettes, layout. Les vues n'en sont que des
  projections.
- Vues : la **liste/arbre** des ressources et de leurs liens est
  l'interface principale, entièrement au clavier (un lien = choisir une
  cible dans un select) ; le **graphe** (`@xyflow/react`) est une vue
  synchronisée ; l'éditeur du nœud sélectionné réutilise `crd-form`
  (`fields`, `schema-utils`, `buildManifest`, `CrdChoices`), un seul
  moteur schéma → formulaire.
- Tests : unitaires sur le modèle pur (aller-retour import/export,
  résolution, validation, recettes), gate de couverture à 80 % ; revue
  `accessibilite-check` (RGAA) et `eco-check` sur le diff. Pas d'e2e
  navigateur en V1.

#### Import d'un cluster (ajouté le 2026-10-22)

`kubectl get <kinds> -A -o yaml` s'importe tel quel : les `List` sont
dépliées, les champs posés par le serveur retirés (`status`, `uid`,
`resourceVersion`, `managedFields`, annotation last-applied…), un `Secret`
devient un objet externe avec ses clés et jamais ses valeurs, un objet de
même identité (kind, namespace, nom) est remplacé. Plusieurs fichiers en
un import (un dossier GitOps). Vérifié sur la doc de weebo-forgejo : liens
et Secrets résolus, aucune valeur ni champ serveur dans l'export ou le
brouillon.

#### Kinds schématiques (ajouté le 2026-10-22)

Un opérateur veut dessiner dans l'IDE ce que Kubernetes ne connaît pas :
le cluster et ses méthodes d'accès (proxyAuthK8s), les sources upstream
d'un registre (batlehub). Décidé :

- **Déclarés par l'opérateur, en Rust** : `weebo_kit_api::links::schematic::<S>(group,
  Scope, &roles)` lit le schéma du type `S` (le `spec`, mêmes marqueurs
  qu'une CRD, retirés du schéma publié) ; `crdgen` l'écrit dans
  `links.json`, section `schematic: { <Kind>: { group, scope, schema,
  references, produces } }` (`version` reste 1 : un lecteur plus ancien
  ignore la clé). Le groupe est celui de l'opérateur
  (`schematic.<groupe>`), la version toujours `v1`.
- **Publiés comme des kinds** : `weebo-crd-docs` écrit leur
  `<kind>.schema.json` et une entrée `index.json` `schematic: true`, sans
  page ; `CrdBuilder` les ignore. Leurs liens sont vérifiés comme ceux
  d'une CRD (chemins depuis la racine : `spec.…`).
- **Éditables et liés** : dans l'IDE, des ressources comme les autres
  (formulaire, références dans les deux sens), tout nom accepté (une URL),
  bordure pointillée et « (schematic) » dans le texte.
- **Dérivés des champs** : une production peut faire des objets
  schématiques. Le marqueur `x-weebo-produces` gagne `each` (un chemin sous
  le champ, pour un champ libre comme le `config` de batlehub), un nom `.`
  désigne l'item lui-même (une URL), et sous un objet libre
  (`x-kubernetes-preserve-unknown-fields`) tout chemin est accepté. Deux
  objets schématiques produits de même identité ne font qu'un nœud
  (`alsoFrom`), fusionné dans la ressource ajoutée à la main qui l'a : pas
  de doublon pour un upstream partagé, et on le complète à la main.
- **Export à part** : jamais dans `manifests.yaml` ni le zip ; un
  `schematic.yaml` téléchargeable, qui se réimporte par le même chemin.

#### Release

Mineure (0.x) : le template `crdgen` change (chaque opérateur régénère et
publie `links.json`), le paquet gagne une dépendance et `index.json` des
champs. Pour les opérateurs, reprendre le `crdgen` du template et ajouter
`x-weebo-keys` à leurs `secretTargets` ; tant qu'ils ne l'ont pas fait,
le site du kit liste leurs kinds sans lien.

#### Ordre de réalisation

1. `weebo_kit_api::links` (marqueur, `of`, test anti-fuite) et `crdgen`
   du template (fait, à publier).
2. `weebo-crd-docs` publie `links.json` ; `CrdLinks` dans sa config
   (fait, à publier : chemins vérifiés contre la CRD, `produces` avec
   `name` en liste de champs, le premier renseigné gagnant, et `keys`).
3. Le modèle TS pur et ses tests (fait, à publier) :
   `@weebo/operator-docs/crd-ide/model` (`src/crd-ide/model/`). Catalogue
   (schémas + `links.json`, plusieurs sites), projet (ressources
   `editable`/`readOnly`/`unknown`, externes, namespace par défaut), graphe
   recalculé (nœuds produits, emplacements de référence résolus par kind,
   nom et namespace), validation (schéma, trous `<…>`, références,
   `sharedWith`, doublons), import/export YAML dans l'ordre des
   dépendances, un fichier par ressource, zip. Écarts au plan :
   - **pas d'`elkjs`/`dagre`** : une colonne par profondeur de dépendance
     suffit à un graphe de quelques dizaines de nœuds, sans dépendance ;
   - **zip « stored » écrit à la main** (CRC32, ~80 lignes) : des
     manifestes de quelques Ko ne justifient pas une lib de deflate ;
   - `weebo-crd-docs` publie la **version de stockage** (et non
     `versions[0]`) et la liste `served` des versions servies (Q19) ;
   - raccourci : le `kind` d'un `InstanceRef` (`Instance`/`LocalInstance`)
     ne restreint pas la cible, la résolution accepte tous les kinds du
     lien ;
   - tests : `node --test` sur les sources TS (types retirés par Node 24,
     `test/ts-resolve.mjs` pour les imports sans extension), gate de
     couverture à 80 % sur `src/crd-ide/**` dans `npm run check`.
4. Les vues : liste, éditeur (`crd-form`), graphe (fait, à publier).
   `<CrdIde />` dans `docsMdxComponents(choices, { recipes, sources })`,
   derrière `LazyCrdIde` (`next/dynamic`, `ssr: false`) : le code de l'IDE
   ne charge que sur sa page, `GraphView` (et `@xyflow/react` 12.12) est un
   second import dynamique, chargé avec la première ressource du projet
   (le choix liste / graphe a été retiré le 2026-10-22 : le graphe est
   toujours affiché au-dessus de la liste, qui garde chaque action). Colonnes en
   container query (la largeur de la colonne de doc, pas de la fenêtre).
   Vérifié dans un build du site (copie jetable sous `target/`) piloté par
   Playwright : recette, liens, graphe, zip, brouillon rechargé, pas de
   défilement horizontal à 390 px. Corrigé au passage dans `crd-form` :
   un `kind` à enum n'est une union discriminée que si une de ses valeurs
   est un champ frère — `InstanceRef` en était une pour `CrdForm`, qui
   retirait `instanceRef` du manifeste. Revues `accessibilite-check` et
   `eco-check` appliquées : la liste reste affichée en vue graphe, focus
   déplacé après une suppression et vers l'éditeur depuis un problème,
   nom de la ressource dans chaque select de lien ; YAML mémoïsé et
   coloration différée, brouillon écrit 500 ms après la dernière frappe
   (et à `pagehide`), layout recalculé au seul changement du graphe. Les
   recettes passent en prop de `<CrdIde recipes={…} />` dans la page : dans
   `docsMdxComponents`, leur YAML partirait avec chaque page.
5. Le site du kit : agrégation au build, catalogue tiers, recettes
   croisées (fait, à publier). Décidé le 2026-10-21 en cours de route :
   - **Q23 — schémas chargés à la demande** : `index.json` porte
     l'`apiVersion` de stockage et les versions servies ; le catalogue de
     l'IDE ne charge au départ que les index et les `links.json` (14
     fichiers sur le site du kit), le schéma d'un kind arrive avec sa
     première ressource (`withSchemas`, `missingSchemas`). Sans schéma, une
     spec n'est ni vérifiée ni élaguée. Les schémas des stores ESO et des
     issuers cert-manager pèsent 255 à 393 Ko chacun, 1,36 Mo pour six
     kinds.
   - **Q24 — les Secrets produits par les opérateurs** : `SecretTarget`
     porte `x-weebo-produces` (un Secret par cible, aucun pour une cible
     `vault`), un champ déclare les clés écrites avec `x-weebo-keys`
     (`#[schemars(extend("x-weebo-keys" = ["token"]))]`). `links::of`
     renvoie `{ references, produces }`, le `links.json` de `crdgen` a la
     forme publiée. Une production a `each`, `namespace`, `keyNames`,
     `unless`. À faire dans chaque opérateur : `x-weebo-keys` sur ses
     `secretTargets` (sans, les clés de ses Secrets sont inconnues et
     `missing-key` ne vérifie rien).
   - **Q25 — opérateurs sans `links.json`** : leurs kinds sont listés sans
     lien jusqu'à leur release sur ce kit.
   - **Q26 — catalogue commité** : `site/crd-catalog.config.mjs` (les 5
     opérateurs à leur tag, ESO v2.12.0 : ExternalSecret, SecretStore,
     ClusterSecretStore ; cert-manager v1.21.2 : Certificate, Issuer,
     ClusterIssuer, avec leurs `CrdLinks`), `task site:catalog` (réseau)
     écrit `site/crd-catalog/`, `scripts/crd-schemas.mjs` (`predev`/
     `prebuild`, hors ligne) génère `public/crd-schemas/<nom>/` avec
     `weebo-crd-docs` en mode `pages: false`.
   - Page `/docs/ide` et deux recettes croisées (`site/src/lib/recipes.ts`) :
     Forgejo avec son token depuis ESO et sa CA depuis cert-manager ; Angos
     avec la connexion à l'UI par une application Authentik. Vérifiées dans
     le navigateur : seuls les trous restent signalés.

### À faire — les kinds schématiques sur tous les opérateurs (décidé le 2026-10-22)

Les kinds schématiques (voir « Kinds schématiques » dans la section de
l'IDE) servent à dessiner ce qui vit hors du cluster. Décidé : un
**vocabulaire partagé** défini par le kit, et **ProxyAuthK8S** rejoint le
kit (`weebo-kit-api`, docs à jour, catalogue du site).

#### Le vocabulaire partagé

Un même objet extérieur (un issuer OIDC, un registre amont) n'est un seul
nœud que si tous les opérateurs le déclarent du même kind. Le kit fournit
donc des kinds neutres, groupe `schematic.weebo.io` : leurs types de spec
dans `weebo_kit_api::links` (doc comments = descriptions), une fonction
qui les déclare tous (le `crdgen` du template l'appelle). Portée `Cluster`
pour tout ce qu'une URL nomme : un nœud produit d'un kind namespacé prend
le namespace de son producteur, deux registres de deux namespaces ne
fusionneraient pas.

| Kind | Nom du nœud | Produit par | À la main |
| --- | --- | --- | --- |
| `OidcIssuer` | URL de l'issuer | ProxyAuthK8S, angos, authentik | forgejo, stalwart |
| `UpstreamRegistry` | URL | batlehub, angos | — |
| `VaultServer` | adresse | forgejo, authentik, stalwart | — |
| `HttpEndpoint` | URL | forgejo, angos, authentik | — |
| `DnsName` | domaine ou hôte | authentik, stalwart, batlehub, angos | — |
| `S3Bucket` | bucket | angos | batlehub |
| `Database` (Namespaced) | — | — | batlehub |

Un kind propre à un seul opérateur reste dans son groupe
(`schematic.<groupe de l'opérateur>`).

#### Kit (v0.24.0, mineure) — publié le 2026-10-22

Fait le 2026-10-22, vérifié dans une copie jetable du site (`target/`,
les kinds du template rendu plus deux productions de test) pilotée par
Playwright à 1400 et 390 px : groupes du sélecteur, nœuds schématiques
(pointillés, « (schematic) »), issuer calculé fusionné dans celui ajouté à
la main, manifestes sans les objets schématiques, `schematic.yaml`
téléchargé, aucune erreur console, pas de défilement horizontal. Écart
trouvé et corrigé : une URL écrite avec son slash (`https://h/`) suivie de
`/…` dans le gabarit donnait `https://h//…` ; un trou suivi d'un `/` perd
le slash final de sa valeur.


- Publier les kinds schématiques (`links::schematic`, `each` du marqueur,
  nom `.`, `schematic.yaml`).
- Le vocabulaire partagé ci-dessus, et `crdgen` (template) qui le déclare.
- Le catalogue de l'IDE accepte un kind schématique déclaré par plusieurs
  sources (le premier gagne) ; un kind de CRD en double reste une erreur.
- `unless.equals` compare aussi un booléen ou un nombre (sa forme texte).
- Les noms d'un kind nommé par une URL sont normalisés (slash final
  retiré) : `…npmjs.org` et `…npmjs.org/` sont un nœud.
- **Nom calculé** (décidé le 2026-10-22, pour l'issuer authentik) : une
  production gagne `template`, le nom quand aucun champ de `name` n'est
  renseigné — `name` porte donc la surcharge de l'utilisateur, le gabarit
  le calcul. `{chemin}` relatif à l'item, `{/chemin}` depuis la racine de
  la ressource, `{/réf>chemin}` lu sur la ressource que désigne la
  référence `réf` (une des références du kind ; non renseignée, la seule
  ressource candidate, comme le défaut du kit). Un trou → pas de nœud.
  Marqueur `x-weebo-produces` et `links.json` : `template`, `name` peut
  être vide s'il y en a un.
- Vérifié dans un build du site (Playwright, 390 px) sur les kinds du
  template rendu.
- Les recettes croisées (un cluster ProxyAuthK8S, ses méthodes d'accès et
  son issuer ; batlehub et angos sur les mêmes registres amont) viennent
  après les releases des opérateurs : le site ne charge que leurs tags.

#### Opérateurs, dans l'ordre

Pour chacun : kit v0.24 (crates, `@weebo/operator-docs`, `crdgen` repris
du template), marqueurs et kinds propres, `task recu`, release, puis son
tag dans `site/crd-catalog.config.mjs`.

1. **batlehub** (pilote) : `UpstreamRegistry` produit par
   `BatleHubRegistry spec.config` (`each: upstreams[]`, nom `.`) et par
   `BatleHubLocalInstance spec.config` (`registries[].upstreams[]`) ;
   `DnsName` par `spec.managed.ingress.host` ; `Database`, `S3Bucket` à la
   main. Limite : ce qui vient de `configFrom`/`configLayers` (TOML de
   ConfigMaps et Secrets) reste invisible.
2. **ProxyAuthK8S** : d'abord rejoindre le kit — dépendance
   `weebo-kit-api` (kube 4, k8s-openapi, schemars 1 : compatibles),
   `crdgen` qui écrit `deploy/crd/*.yaml` et `links.json` comme le
   template, docs de `@weebo/operator-docs` 0.20.0 à 0.24 avec une page
   `ide.mdx`. Puis : `KubeCluster` (son groupe, `schematic.weebo.si.rs`)
   produit par `spec.service.ExternalService.url` ; une vraie référence
   vers le `Service` de `spec.service.KubernetesService` ;
   `AccessMethod` (son groupe, à la main : `type` dashboard, plugin
   kubectl, serveur OAuth ou direct, `proxyRef` → `ProxyKubeApi`, `group`
   OIDC exigé) ; `OidcIssuer` produit par `spec.auth_config.jwt[].issuer.url`
   et `spec.auth_config.oidc_provider.issuer_url` ; références vers les
   Secrets et ConfigMaps de `cert`, `client_cert`,
   `virtual_apis[].list_fallback_token`. Ajouté au catalogue du site.
3. **angos** : `UpstreamRegistry` par `AngosRepository spec.upstreams[]`
   et `spec.downstreams[]` (nom `url`) ; `S3Bucket` par
   `spec.storage.s3` et `spec.storage.metadata.s3` ; `OidcIssuer` par
   `spec.auth.oidc[]` et `spec.auth.kubernetes`, `spec.ui.oidc.provider`
   y pointe ; `HttpEndpoint` par `spec.scan`, `spec.auth.webhooks[]`,
   `spec.eventWebhooks[]`, `spec.tracing` ; `DnsName` par
   `AngosIdentity spec.registryHost`.
4. **authentik** : `OidcIssuer` produit par `spec.provider.oauth2`, nommé
   par le nouveau champ `issuerUrl` (surcharge, optionnel ; l'opérateur
   l'écrit aussi en statut) sinon calculé :
   `{/spec.instanceRef>spec.url}/application/o/{/spec.slug}/` ;
   `HttpEndpoint` par `spec.provider.proxy.internalHost`,
   `DnsName` par `AuthentikBrand spec.domain` et
   `spec.provider.proxy.externalHost` ; `VaultServer` par
   `spec.secretStore.vault.address`.
5. **forgejo** : `GitRemote` (son groupe) par
   `ForgejoRepository spec.migrate.cloneAddr` ; `HttpEndpoint` par
   `ForgejoWebhook spec.url` ; `VaultServer`. `loginSourceId` est un
   numéro : l'IdP s'ajoute à la main.
6. **stalwart** : `DnsName` par `StalwartDomain` (`spec.name`, sinon
   `metadata.name` : par les `links` de sa config docs, le marqueur ne lit
   pas `metadata`) et `spec.aliases[]` ; `HttpEndpoint` par
   `spec.submission` ; `VaultServer`.

#### Avancement (2026-10-22)

Publiés sur le kit v0.24.0 : weebo-batlehub v0.5.0, weebo-angos v0.3.0,
weebo-authentik v0.21.0, weebo-forgejo v0.7.0, weebo-stalwart v0.3.0 ;
ProxyAuthK8S v0.3.0 (#289, puis sa branche `release/v0.3.0`, #290 ; son
job `integration` passe à 45 min, #291 : un cache de build froid, après
un bump de version, l'avait coupé deux fois à 30). Le catalogue du site les épingle à ces tags. Vérifié
sur le modèle de l'IDE avec les schémas générés : l'issuer calculé d'une
application authentik (URL d'instance écrite avec son slash), celui d'un
`ProxyKubeApi` et celui d'une instance Angos (sans slash) sont un seul
nœud, les kinds partagés sont une fois au catalogue.

Écarts au plan :

- kit, dans la même release : un marqueur `x-weebo-produces` sur un champ
  liste produit un objet par item (`extraHosts`), comme `x-weebo-link` ;
- angos : `downstreams[].url` est un `HttpEndpoint` (une cible de
  réplication, pas un registre amont) ;
- authentik : `proxy.externalHost` est une URL, dessinée en
  `HttpEndpoint` ; `proxy.internalHost` (un Service du cluster) n'est pas
  dessiné. `spec.provider.oauth2.issuerUrl` est une vraie surcharge :
  écrite en `AUTHENTIK_URL` à la place de l'issuer calculé, et l'issuer
  écrit est enregistré en `status.issuerUrl` ;
- stalwart : `submission.host` est un `DnsName` (un hôte SMTP, pas un
  `HttpEndpoint`) ; le nom du domaine passe par les `links` de sa config
  docs (`spec.name`, sinon `metadata.name`) ;
- ProxyAuthK8S : ses références (Secrets, `Service` d'un
  `KubernetesService`) restent dans sa config docs, les productions sont
  des marqueurs sur ses types `crd` (sans dépendance), `weebo-kit-api`
  n'entre que dans son `crdgen` (`--links`, `publish = false`, la source
  git du kit autorisée dans `deny.toml`).

#### Limites acceptées

- Les valeurs tenues dans des Secrets (URL Redis d'angos, DSN de
  batlehub, `issuerFrom` d'angos) ne seront jamais vues par l'IDE.

### Manques relevés par weebo-notify (2026-10-29, non cassant)

Trouvés en créant weebo-notify depuis le template (kit v0.24.0) ; chacun
avait été contourné par une copie dans l'opérateur.

- **Lints du template** : un projet rendu n'avait ni `[workspace.lints]`,
  ni `clippy.toml`, ni `[lints] workspace = true` dans ses crates, alors
  que les lints weebo-good-practice valent pour tout workspace. Le
  template porte désormais les mêmes que le kit (et weebo-angos) ; ce qui
  les enfreignait est corrigé (`let _ =` sur `install_default` et sur
  l'attente du serveur, `expect` de sérialisation remplacé par
  `kube::Error::SerdeError`), ou excusé là où une panique est légitime
  (`#[expect]` sur l'URL de la gateway, vérifiée par `new` ; doublure
  `test_support` et helpers des tests d'intégration, comme dans le kit).
  Vérifié : `task template:render` puis `clippy --all-targets -D warnings`
  sur le rendu.
- **Avertissements d'admission** : `Admission` ne savait qu'admettre ou
  refuser. Nouvelle méthode fournie `admit_with_warnings` →
  `Result<Vec<String>, Denial<_>>` (par défaut `admit`, sans
  avertissement), celle qu'appelle `/validate` ; les avertissements d'un
  objet admis vont dans `warnings` de la réponse (affichés par `kubectl`),
  aucun champ quand il n'y en a pas. Compteur de décisions inchangé ; un
  implémenteur existant ne change ni de build ni de comportement.
- **Teardown d'envtest** : `env.Start()` de controller-runtime ne
  s'arrête pas quand l'installation des CRD échoue, et le crate `envtest`
  ne garde alors aucun handle : `kube-apiserver` et `etcd` restaient
  lancés. Le testkit ne donne plus les CRD à envtest : il démarre le
  control plane nu, le possède (`Drop` l'arrête, panique comprise), puis
  installe les CRD lui-même (fichiers `.yaml`/`.yml`/`.json`, documents de
  kind `CustomResourceDefinition`, comme envtest) et attend qu'elles soient
  servies par la découverte. Nouveau `EnvTestCluster::try_start` (l'échec
  rendu au lieu d'une panique). Reste hors d'atteinte : un échec à
  l'intérieur du démarrage d'envtest (namespace `default` jamais créé…).
  Tests envtest `tests/envtest_teardown.rs` (ignorés par `cargo test`,
  `task test:envtest`, dans `test:all`).
- **`KUBEBUILDER_ASSETS` sans réseau** : avec le téléchargement activé,
  envtest lit son index de releases même quand les binaires sont là. Si
  `KUBEBUILDER_ASSETS` nomme un répertoire qui contient `kube-apiserver` et
  `etcd`, le testkit les prend tels quels, sans téléchargement ni appel
  réseau ; défini sans eux, comportement d'avant (avec un message).

weebo-notify peut retirer ses copies au `kit:sync` suivant : son
`/validate` (`admit_with_warnings`) et son `testkit/src/envtest.rs`.

### À faire — une variante « gérée » du template

Le template ne connaît que la forme pilotée par API (une `RemoteGateway`
vers le produit) ; or weebo-batlehub, weebo-angos et weebo-notify
déploient tous leur produit avec la couche « workload géré » (v0.17.0 :
`OwnedObjects`…), et chacun a remplacé la gateway à la main. Piste : une
variante (placeholder de cargo-generate, ou second template) qui rend une
`<Product>LocalInstance` déployée par `OwnedObjects` et des objets liés à
leur instance, au lieu de la gateway HTTP — vérifiée par `template:check`
comme l'autre.
