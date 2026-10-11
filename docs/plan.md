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
| `weebo-kit-application` | application  | kit-api                       | ports `SecretStore`/`SecretStoreFactory`, helpers d'émission de secrets, `FakeSecretStore`       |
| `weebo-kit-adapters`    | outbound     | kit-application, kube, vaultrs| `K8sSecretStore`, `VaultSecretStore` + `VaultLogins`, `RoutingSecretStore`, `read_secret_key`   |
| `weebo-kit-runtime`     | inbound/bin  | kit-domain, kit-api, kube     | télémétrie, leader election, `Shutdown`, serveur webhook TLS, métriques, conditions + Events    |
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
  `metadata`), et un CR namespacé reste sous `<pathPrefix>/<namespace>/`.
- `emit::check_target(s)` : un CR namespacé n'écrit que dans son
  namespace (`TargetViolation`), vérifié avant tout appel et par les stores.
- `RoutingSecretStore::lazy` : Vault n'est contacté qu'à la première cible
  Vault ; `VaultLogins` verrouille par instance et rejoue un échec 15 s.
- **Leader election** réécrite (plus de `kube-leader-election`) : écritures
  du Lease gardées par `resourceVersion` (jamais deux leaders), expiration
  jugée sur l'horloge monotone locale ; `leader::is_leader()` pour une jauge.
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
  `OtherInstance`), `require` → `RefError` neutre (`describe` pour le
  message), `same_instance` (un `instanceRef` vide = la seule instance).
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
  (placeholder `product`, en CamelCase). Workspace hexagonal câblé sur
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
