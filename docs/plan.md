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

### Itération 3 — généraliser l'application

Candidats identifiés dans forgejo, à rendre génériques une fois
authentik aligné (leur forme dépend de l'identifiant distant : `i64` +
nom chez forgejo, `String` chez authentik) :

- résolution des `*Ref` (`ReferenceReader`, `RefState`, `require`,
  `recorded_name`) et le `KubeClusterReader` qui l'implémente ;
- admission : `Governed`, `enforce`, `PolicySource`, le routeur
  `/validate` du webhook ;
- résolution de l'instance (`InstanceResolver`, `select` par
  `instanceRef`/instance par défaut) et le squelette des gateway
  factories ;
- squelette de controller (`Ctx`, `error_policy`, `requeue_after`,
  annotations communes `allow-disruptive-update`/`rotate-secret`/
  `import-pending`) ;
- squelette de l'importer (création en pause, status amorcé, reprise).

### Hors Rust (plus tard)

- Chart Helm : un *library chart* (RBAC, webhook + cert-manager, PDB,
  ServiceMonitor) consommé par le chart de chaque opérateur.
- Taskfile : includes distants (`lint`, `audit`, `doctor`, `coverage`).
- CI : workflows réutilisables (`workflow_call`) pour lint/test/audit.
- Gabarit `cargo-generate` d'un nouvel opérateur : workspace hexagonal
  câblé sur le kit, une CRD `XInstance` + une ressource exemple.
