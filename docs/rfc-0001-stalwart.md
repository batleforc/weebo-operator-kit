# RFC 0001 — Stalwart, messagerie interne, et l'opérateur weebo-stalwart

- **Dépôts :** weebo-si (déploiement), `weebo-stalwart` (nouvel opérateur,
  généré depuis le template de ce kit). La RFC vit ici, dans
  weebo-operator-kit, depuis la revue du 2026-10-18.
- **Base :** weebo-operator-kit v0.18.x (en cours : `weebo-kit-domain::drift`,
  `DriftCorrected`, `remote_writes_total`, alerte `RepeatedDrift` du chart),
  Stalwart v0.16.27 (2026-10-18, source `main` `aa47d8e`), weebo-authentik
  0.18.1, OpenBao (`mv`)
- **Statut :** Draft. Auteurs : batleforc. Créée le 2026-10-18, mise à jour
  le 2026-10-18 (revue appliquée), non décidée.
- **Release visée :** weebo-stalwart v0.1.0 (comptes d'envoi), v0.2.0
  (boîtes, groupes, listes). Aucun changement du kit n'est requis.
- **Brique :** `weebo-si:2.argo/helm/mail`, github.com/batleforc/weebo-stalwart
- **Ligne de left-todo :** « Setup Stalwart as an internal mail system for the cluster… »

## Résumé

On déploie [Stalwart](https://stalw.art/) v0.16.27 dans le namespace `mail`
comme seul serveur mail du domaine interne `weebo.poc` : les apps du cluster
y soumettent leurs mails, les utilisateurs Authentik y ont une boîte, et rien
ne sort du cluster. Un nouvel opérateur, `weebo-stalwart`, généré depuis le
template de weebo-operator-kit, porte le cycle de vie de ce qui vit dans
Stalwart :

- le domaine ;
- les comptes d'envoi des apps, avec leurs credentials publiés dans Vault ;
- les boîtes des utilisateurs (`StalwartAccount`), créées et verrouillées
  par CR ;
- les groupes et les listes.

Les App Passwords restent en libre-service dans la WebUI en v1 (D14) : les
créer pour autrui exige `impersonate`.

C'est le même contrat que weebo-forgejo : CR déclaratifs, `Ready` dans
`status.conditions`, `secretTargets`, allow-list par namespace. Les
écritures suivent la RFC 0003 : lire, comparer, n'écrire que la dérive.

L'opérateur ne connaît que Stalwart. **Le lien avec Authentik** (un
`AuthentikUser` donne un `StalwartAccount`, son retrait le verrouille) est
le rôle d'une **autre brique**, qui lie les opérateurs entre eux :
[RFC 0002](https://github.com/batleforc/weebo-si/blob/develop/0.docs/content/docs/rfc/0002-liaison.md),
par en-têtes `link.weebo.io`, un petit contrôleur `weebo-link` et des
`ValidatingAdmissionPolicy`. Celle-ci ne fixe que le contrat que cette
brique consomme : les CR `stalwart.weebo.io`.

Cette RFC concerne quiconque opère weebo-si, ajoute une app qui envoie du
mail, ou écrit un opérateur sur le kit.

## Motivation

### Aujourd'hui

- **Aucune app n'envoie de mail.** Authentik (récupération de compte,
  enrollment), Forgejo (notifications, reset), les notifications Argo CD et
  les alertes de monitoring n'ont aucune config SMTP. Un grep `smtp|mailer`
  sur `weebo-si:2.argo`, `weebo-si:2.helm` et `weebo-si:2.terra` ne renvoie
  rien. Les adresses en `@weebo.poc` qui existent (`forgejo-admin@weebo.poc`,
  les emails des `AuthentikUser`) ne mènent nulle part.
- **Les brancher sur un SMTP externe** (Gmail, un relais OVH) ferait sortir
  du cluster des mails qui contiennent des liens de reset et des jetons
  d'invitation, vers une infra qu'on ne contrôle pas. C'est contraire à
  l'objectif de weebo-si, où tout le cycle dev reste interne.
- **Stalwart seul ne suffit pas.** Sa directory OIDC (contre Authentik)
  authentifie les utilisateurs, mais la
  [doc](https://stalw.art/docs/auth/backend/oidc/) en donne elle-même les
  limites :
  1. *Pas de compte avant le premier login.* Un compte n'existe qu'après une
     première authentification (JIT). D'ici là, un mail vers
     `alice@weebo.poc` est rejeté, faute de destinataire local.
  2. *Pas de déprovisioning.* Un utilisateur supprimé ou désactivé dans
     Authentik garde sa boîte, qui continue de recevoir.
  3. *Clients lourds.* Thunderbird, Apple Mail et Outlook ne font pas
     OAUTHBEARER avec un IdP tiers. La réponse documentée est l'App Password.

  Les deux premiers points ont une réponse officielle, SCIM, mais elle est
  réservée à l'édition Enterprise. Aucun des trois ne couvre les comptes
  d'envoi des apps.

### Une fois en place

- Chaque app qui envoie du mail a un compte d'envoi dédié. Ses credentials
  sont dans `mv/<app>/smtp`, où son ExternalSecret les lit, sans copier-coller.
- Un `StalwartAccount` donne une boîte qui reçoit dès sa création, sans
  attendre un premier login. Passé en `state: Locked`, la boîte n'accepte
  plus ni login ni réception. Qui crée et verrouille ces CR (à la main, ou la
  brique de liaison depuis Authentik) n'est pas l'affaire de l'opérateur.
- Une CiliumNetworkPolicy garantit qu'aucun pod de `mail` ne joint
  l'extérieur, quelle que soit la config de Stalwart.
- Chaque objet a un état lisible : `kubectl get stalwartserviceaccounts -A`
  donne `Ready` et la raison, comme pour forgejo.
- Une réconciliation sans changement ne fait que des lectures :
  `weebo_stalwart_remote_writes_total{cause="drift"}` reste à 0 au repos.

C'est ce qu'on vérifiera au passage en `Implemented`.

### Objectifs et non-objectifs

| Objectif | Non-objectif |
| --- | --- |
| Mail interne `weebo.poc`, app → user et user ↔ user | Recevoir du mail d'Internet (MX public) |
| Comptes d'envoi des apps, credentials dans Vault | Envoyer vers Internet (relais sortant) |
| Boîtes des users (`StalwartAccount`), création et verrouillage | Webmail tiers (Roundcube, SnappyMail…) |
| Opérateur sur le kit, conventions des autres opérateurs | Déployer Stalwart depuis l'opérateur (mode *managed*) |
| Un seul nœud, une réplique | Cluster Stalwart multi-nœuds |
| Un contrat CR stable, consommable par la brique de liaison | Lire les CR d'un autre opérateur (`AuthentikUser`…) : c'est la brique de liaison |
| N'écrire dans Stalwart que ce qui diffère (RFC 0003) | Créer des App Passwords pour autrui en v1 (D14) |

## Explication pour l'exploitant

### Ce qui est installé

L'Application Argo `mail` (`weebo-si:2.argo/app/templates/mail.yaml`,
projet `dev-base`), nouveau chart `weebo-si:2.argo/helm/mail`. Les waves
suivent celles de forgejo (SecretStore sans wave, ExternalSecrets et
Certificate en 1, CNPG en 2, workload et opérateur en 3, instance et policy
en 4, CR en 5) :

| Wave | Objet | Rôle |
| --- | --- | --- |
| 0 | SecretStore `cred-store` | lecture de `mv` (rôle `mail-read`) |
| 1 | ExternalSecrets, Certificate `stalwart-tls` | `mv/mail/{pg,s3,admin,auth}` vers des Secrets de `mail` ; TLS des listeners (CA interne) |
| 2 | Cluster CNPG `stalwart` | datastore |
| 3 | StatefulSet `stalwart`, Services, IngressRoute, CiliumNetworkPolicy | le serveur |
| 3 | Application `weebo-stalwart` | l'opérateur, dans le namespace `weebo-stalwart` |
| 4 | `StalwartInstance` `default`, `StalwartNamespacePolicy` `weebo-allow` | connexion et allow-list |
| 5 | `StalwartDomain` `weebo.poc` (cluster) | le domaine |
| PostSync + CronJob | Job `stalwart-bootstrap` | config du serveur, compte et ApiKey de l'opérateur, revalidés chaque jour (D17) |

Tant que le Job n'a pas tourné une fois, la `StalwartInstance` est
`Ready: False` (`TokenSecretMissing`) et passe `Ready` seule ensuite, comme
la `ForgejoInstance`.

Les comptes d'envoi sont déclarés **dans le chart de chaque app**, à côté de
son ExternalSecret, comme les `AuthentikApplication` aujourd'hui.

### Brancher une app (exemple : Forgejo)

```yaml
# weebo-si:2.argo/helm/forgejo/templates/mail/service-account.yaml
apiVersion: stalwart.weebo.io/v1alpha1
kind: StalwartServiceAccount
metadata:
  name: forgejo
  namespace: forgejo
  annotations:
    argocd.argoproj.io/sync-wave: "5"
    argocd.argoproj.io/sync-options: SkipDryRunOnMissingResource=true
spec:
  domainRef: { name: weebo.poc }   # StalwartDomain cluster-scoped
  name: forgejo                 # forgejo@weebo.poc
  aliases: [noreply]            # noreply@weebo.poc, utilisable en From
  receive: false
  secretTargets:
    - backend: vault
      name: smtp                # mv/forgejo/smtp ; seul nom Vault admis (D15)
```

`mv/forgejo/smtp` contient alors :

| Clé | Valeur |
| --- | --- |
| `host` | `stalwart.mail.svc.cluster.local` |
| `port` | `587` |
| `security` | `starttls` |
| `username` | `forgejo@weebo.poc` |
| `password` | 32 caractères générés par l'opérateur |
| `from` | `noreply@weebo.poc` (le premier alias, sinon l'adresse) |

L'ExternalSecret de forgejo les injecte dans `[mailer]`.

```console
$ kubectl -n forgejo get stalwartserviceaccount
NAME      ADDRESS             READY   REASON        AGE
forgejo   forgejo@weebo.poc   True    Reconciled    3m
```

Et quand ça ne marche pas :

```console
$ kubectl -n batlehub get stalwartserviceaccount
NAME       ADDRESS              READY   REASON               AGE
batlehub   batlehub@weebo.poc   False   NamespaceNotAllowed  10s
```

L'admission l'aurait déjà refusé, avec le même message. Le reconciler
revérifie pour le cas où le webhook a été contourné.

### Donner une boîte à un utilisateur

Un `StalwartAccount` dans `mail`. En régime normal, c'est la brique de
liaison qui le pose depuis l'`AuthentikUser` ; en attendant, ou pour un cas
isolé, on le déclare à la main :

```yaml
apiVersion: stalwart.weebo.io/v1alpha1
kind: StalwartAccount
metadata:
  name: alice
  namespace: mail
spec:
  domainRef: { name: weebo.poc }
  name: alice                  # = preferred_username Authentik
  quota: 2Gi                   # quotas.maxDiskQuota = 2147483648
```

La boîte reçoit immédiatement. Alice se connecte à la WebUI
(`https://mail.weebo.poc`) par Authentik : la JIT OIDC retombe sur ce compte
(même local-part, même domaine). Pour Thunderbird, elle crée elle-même un
App Password dans la WebUI et le colle dans son client (D14). Le secret ne
transite ni par Git ni par Vault.

Quand Alice part, son `StalwartAccount` passe en `state: Locked` (voir
*Verrouillage*), par la brique de liaison ou à la main. Ses App Passwords
cessent de fonctionner avec le reste, et la boîte reste sur disque jusqu'à
un `Delete` explicite.

## Design

### Compatibilité avec weebo-operator-kit

Règle de cette RFC : **rien n'est réinventé à côté du kit**. Le dépôt est
généré par
`cargo generate --git https://github.com/batleforc/weebo-operator-kit --tag v0.18.x templates/operator`
(`product = Stalwart`, d'où le groupe `stalwart.weebo.io`). La v0.18 est
visée, et pas la v0.17 : c'est elle qui apporte le modèle « lire, comparer,
n'écrire que la dérive » de la RFC 0003 (`weebo-kit-domain::drift`,
`DriftCorrected`, `remote_writes_total`, alerte `RepeatedDrift`).

Le dépôt garde le découpage, les noms et les comportements du template. Ce
qui est propre à Stalwart se limite à ce que le kit laisse explicitement à
chaque opérateur (`docs/plan.md`, « Ce qui va dans le kit ») :

- les `#[derive(CustomResource)]` ;
- le catalogue `ReasonCode` ;
- les gateways et leurs ports ;
- les use-cases `reconcile_*` ;
- la factory qui lit l'instance ;
- les valeurs par défaut du schéma (`pathPrefix`).

Le template n'a ni `secretTargets`, ni connexion Vault, ni crate `importer`.
Ces trois pièces sont **reprises de weebo-forgejo**, qui les consomme déjà
depuis le kit : `emit` et `SecretStoreFactory` dans le use-case et la
factory, `crates/importer` pour `weebo-stalwart-importer`.

| Convention du kit | Application ici |
| --- | --- |
| `StalwartInstance` (cluster) + `StalwartLocalInstance` (namespacé, `sharedWith`, Secret du jeton labellé `instance-token`) | Les deux sont gardées telles que le template les génère. Résolution par l'`InstanceDirectory` du kit : `instanceRef` omis = l'unique `StalwartInstance`, sinon l'unique `StalwartLocalInstance` du namespace. |
| `InstanceRef {name, kind, namespace}` et `status.instance` (`BoundInstance`) | Sur chaque kind lié à une instance. Un objet ne change jamais d'instance en silence (`InstanceBindingChanged`). |
| `ObjectRef {name}` : même namespace pour une cible namespacée, cluster-wide pour une cible cluster-scoped | Pas de référence cross-namespace : `StalwartDomain` est **cluster-scoped** (un domaine est une infra d'organisation, comme une instance), et tout le reste se référence dans son namespace. |
| `*NamespacePolicy` cluster-scoped, moteur `allow_list` (namespaces × kinds × dimensions, `Allow`/`Deny`, deny-by-default dès qu'une policy existe) | `StalwartNamespacePolicy`, avec deux dimensions : `domains` (noms des `StalwartDomain`) et `names` (local-parts Stalwart, nom et alias, comme `allowedUsers` chez forgejo). |
| `SecretTarget` / `secretTargets`, cible implicite nommée d'après le CR, `emit::write_all`, `any_missing`, check-and-set Vault, owner sur chaque chemin | Repris de forgejo, pour le seul kind qui émet : `StalwartServiceAccount`. Aucun chemin hors de `<pathPrefix>/<namespace>/` : `pathGrants` n'est pas nécessaire. Une cible Vault s'appelle `smtp` (D15). |
| `secretStore {backend, vault {address, mount, pathPrefix, kubernetesAuthRole, kubernetesAuthMount, caSecretRef, pathGrants}}` sur l'instance | Repris tel quel. `pathPrefix` vaut `weebo-stalwart` par défaut, et weebo-si pose `""` pour écrire `mv/<namespace>/smtp`. |
| `TlsOptions` / `LocalTlsOptions` | `spec.tls` des deux instances. |
| Annotations `<domaine>/force-orphan`, `allow-disruptive-update`, `rotate-secret` (nonce), `import-pending`, `previous-owner` ; label `instance-token` sur le Secret du jeton | Toutes, sous `stalwart.weebo.io/`, avec la sémantique du kit. Aucune annotation de comportement en plus. |
| `status` : `observedGeneration`, `remoteId` (sa présence = ce CR possède l'objet distant), `instance`, `conditions` (`Ready`) | Un `Status` commun à tous les kinds. Deux champs **ajoutés**, absents du kit et du template, repris de forgejo (`crates/api/src/status.rs`) : `lastRotation` (le nonce consommé, lu par `emit::rotation_requested`) et `address` (l'adresse confirmée au dernier passage, D19). |
| Colonnes `Instance` / `URL`, `Ready`, `Reason` | Les mêmes, plus `Address` sur les kinds de compte. |
| Lire, comparer, n'écrire que la dérive (`drift::Drift`, `same_set`, `same_subset`), Event `DriftCorrected`, `remote_writes_total{kind,cause}` | Repris tel quel, champ par champ (voir *Champs comparés*). |
| Jamais d'adoption par nom : un nom pris échoue (`…AlreadyExists`) ; reprise uniquement par l'importer (`import-pending`) après la garde de revendication unique (`RemoteObjectClaimed`) | Repris de forgejo : `StalwartObjectAlreadyExists`, `RemoteObjectClaimed`, `weebo-stalwart-importer`. Le template, lui, lit et adopte par nom (`gateway.read(&desired.name)`) : on s'en écarte (D9). |
| Finalizer, `deletionPolicy: Delete \| Orphan` | Sur tous les kinds distants, `Orphan` seul pour `StalwartDomain` (D18). Le verrouillage est un **état** (`spec.state`), pas une politique de suppression. |
| Runtime : leader election (Lease), `WatchScope`, health, OTLP, `ReconcileMetrics`, `Admission` + `check_vault_targets` + `ScopeGuard` | Utilisés sans surcouche. |
| Chart `weebo-operator-lib`, `tasks/*.yaml`, `checks.yml` / `publish.yml`, `cog.toml` du kit, `@weebo/operator-docs`, `weebo-kit-perf` | Le dépôt les consomme par tag, comme forgejo. Chart publié en `oci://ghcr.io/batleforc/charts/weebo-stalwart`. |

Deux sujets touchent la frontière avec le kit, sans le modifier :

- **Aucun CRD d'un autre opérateur n'est lu.** Comme chaque opérateur du
  kit, weebo-stalwart ne watche que ses propres kinds (et les Secrets de
  ses instances et cibles). Le lien Authentik → Stalwart appartient à la
  brique de liaison (D7), qui n'écrit que des CR `stalwart.weebo.io` et
  passe donc par le même webhook et la même allow-list que n'importe quel
  client.
- **Le socle « workload géré » (v0.17)** n'est pas utilisé en v1 (D1). Le
  jour du mode *managed*, il est consommé tel quel, comme batlehub.

Deux écarts au template sont assumés et signalés au kit : la lecture par
`remoteId` (D9) et le contrôle de version par hash de schéma (D13). Le
premier aligne le template sur la convention qu'il documente lui-même ; il
devrait y remonter. Si le spike montre un autre besoin hors de ces
mécanismes, l'opérateur ne le contourne pas : on ajoute la pièce au kit
d'abord (règle des deux opérateurs), par une RFC du kit.

### Décisions

| # | Décision | Pourquoi |
| --- | --- | --- |
| D1 | Stalwart déployé par weebo-si (chart maison), opérateur sur un serveur existant | La doc Stalwart présente son chart comme un exemple, non publié. Le mode *managed* reste ouvert (*Travaux futurs*). |
| D2 | Template du kit sans retrait : `StalwartInstance` et `StalwartLocalInstance` | `InstanceKind` du kit connaît les deux, et l'`InstanceDirectory` résout les deux. Retirer la locale ferait diverger le schéma de `instanceRef`. weebo-si n'utilise que la cluster. |
| D3 | `StalwartDomain` cluster-scoped | `ObjectRef` du kit ne traverse pas les namespaces. Un domaine partagé par toutes les apps doit donc être cluster-scoped, et c'est ce qui en fait une dimension de l'allow-list. |
| D4 | Client JMAP écrit à la main, sur `POST /jmap` (session `GET /jmap/session`) | Six types `x:*`, trois méthodes chacun. `/api` ne porte que `auth`, `discover`, `account`, `schema`, `token`, `live` (`crates/http/src/api/mod.rs`). Validé par un test de contrat contre l'image. |
| D5 | L'opérateur ne possède que les champs qu'il écrit | La JIT OIDC écrit aussi sur `x:Account` : `description` (claim `name`) et `memberGroupIds` (`crates/common/src/cache/directory.rs`). Les champs possédés sont dans *Champs comparés*. |
| D6 | Les groupes d'un user viennent d'OIDC (`claimGroups`) | Conséquence de D5 : la JIT remplace `memberGroupIds` à chaque login. |
| D7 | Le lien Authentik → Stalwart est hors de l'opérateur, dans une autre brique | Un opérateur par produit : weebo-stalwart ne lit aucun CRD `authentik.weebo.io`, et weebo-authentik n'écrit rien chez Stalwart. La brique de liaison, commune à tous les opérateurs, traduit un objet d'un produit en CR d'un autre. Elle a sa propre RFC. |
| D8 | Départ d'un user = `state: Locked`, jamais une suppression | Un retrait par erreur ne détruit pas de mail. `deletionPolicy` garde la sémantique du kit. C'est la consigne donnée à la brique de liaison. |
| D9 | Lecture par `status.remoteId`, jamais par nom ; sans `remoteId`, création, et un nom pris donne `StalwartObjectAlreadyExists` | Écart assumé au template (`gateway.read(&desired.name)`, qui adopte), aligné sur forgejo et sur « jamais d'adoption par nom » du kit. Un compte JIT existant n'est repris que par l'importer, après la garde de revendication unique. |
| D10 | `tokenSecretRef` = ApiKey Bearer, à permissions `Replace` minimales | Contrat d'instance du kit (un jeton dans un Secret, relu à chaque appel). `stalwart-cli --api-key` (`STALWART_TOKEN`) envoie déjà un Bearer. Repli si le spike l'invalide (Q1) : le même Secret porte `weebo-stalwart:<mot de passe>`, généré et tourné par le même Job (D17), et seule la gateway passe en Basic. Rien dans Vault. |
| D11 | DataStore PostgreSQL (CNPG), BlobStore S3 (RustFS) | Ça reprend les sauvegardes existantes et laisse le StatefulSet jetable. |
| D12 | Egress de `mail` coupé au niveau Cilium | « Rien ne sort » ne dépend pas de la config Stalwart. |
| D13 | Version contrôlée par le hash du schéma : `GET /api/schema` redirige vers `/api/schema/<sha256>`, comparé à la liste des hashes testés, constante de l'opérateur | Aucun endpoint ne renvoie la version : elle n'est que dans `stalwart --version` et la télémétrie, et la bannière SMTP par défaut ne la donne pas (« Stalwart ESMTP at your service »). Le hash identifie exactement le contrat que le client parle. Hash inconnu : `UnsupportedStalwartVersion`, bloquant. |
| D14 | Pas de `StalwartAppPassword` en v1 : App Passwords en libre-service dans la WebUI | Créer un `x:AppPassword` pour un autre compte, c'est poser son `accountId`, donc `impersonate` (`AccessToken::is_member`). Avec `impersonate`, l'opérateur lirait tous les mails. Le libre-service règle aussi la remise du secret : il ne passe ni par Git ni par Vault. Le kind revient en *Travaux futurs*. |
| D15 | Policy Vault du rôle `stalwart-operator` limitée à `mv/data/+/smtp` et `mv/metadata/+/smtp` ; une cible Vault d'un `StalwartServiceAccount` s'appelle `smtp`, sans `path` | Avec `pathPrefix: ""`, le kit adopte un chemin sans owner, et tout ce que Terraform écrit n'en a pas (`mv/mail/admin`, `mv/forgejo/admin`…). Voir l'invariant de *Vault*. La cible implicite (`mv/<ns>/<nom du CR>`) serait refusée par la policy : l'admission la refuse avant, avec un message clair. |
| D16 | Unicité `domainRef + name` entre CR contrôlée au reconcile, comme forgejo | Ni le kit (`weebo-kit-runtime/src/admission.rs`) ni forgejo ne la font à l'admission. Forgejo la fait au reconcile (`ClusterReader::claimants`, `adapters-outbound/src/cluster_reader.rs`) : on reprend ce mécanisme. La porter à l'admission serait une pièce du kit, par sa règle des deux opérateurs. |
| D17 | Jeton de l'opérateur : schéma du Job token de forgejo | `weebo-si:2.argo/helm/forgejo/templates/operator/token.yaml` : PostSync et CronJob quotidien, revalidation du jeton à chaque passage, suppression de l'ancien du même nom avant recréation, écriture dans un Secret Kubernetes (pas Vault). Une ApiKey révoquée est réparée au plus tard le lendemain, un échec entre création et écriture ne laisse pas de clé orpheline. |
| D18 | `StalwartDomain` n'accepte que `deletionPolicy: Orphan` | L'ApiKey n'a pas `sysDomainDestroy` (Q6). Exposer `Delete` sans le droit donnerait un finalizer qui échoue toujours. Un domaine se supprime à la main. |
| D19 | Un `x:*/get` par `remoteId` n'est accepté que si l'adresse renvoyée égale `status.address` | Si les ids sont des numéros de document réattribués (base remise à zéro), un `remoteId` peut désigner un autre compte. Une adresse qui diffère vaut objet absent : on retombe en création, jamais en patch du compte d'un autre. |
| D20 | `Locked` appliqué = `Ready: True`, raison `Reconciled` | C'est l'état voulu, atteint. L'expression `safe` de la RFC 0002 exige `Ready == True` sur un compte `Locked`. |
| D21 | Import d'un compte d'envoi de la phase 0 = rotation forcée | Le mot de passe écrit par Terraform n'a pas d'owner Vault. Le reprendre laisserait deux sources (Terraform et l'opérateur). La ressource Terraform est retirée d'abord, puis l'opérateur régénère et écrit avec son owner. |
| D22 | Ni port 25, ni sonde blackbox, ni alerte en v1 | Tout passe par 587, et la livraison locale ne passe pas par SMTP. weebo-si n'a ni blackbox exporter ni moteur d'alerte (*Exploitation*) : ce sont des briques à créer, chiffrées à part. |

### Contrat

Groupe `stalwart.weebo.io`, version `v1alpha1`. Tous les kinds ont :

- le `Status` commun (`observedGeneration`, `remoteId`, `instance`,
  `conditions`, et selon le kind `address`, `lastRotation`) ;
- les colonnes `Ready` et `Reason` ;
- le finalizer du runtime ;
- les annotations du kit sous `stalwart.weebo.io/`, et le label
  `instance-token` sur le Secret du jeton d'une `StalwartLocalInstance`.

`instanceRef` est l'`InstanceRef` du kit, à la même sémantique que dans le
template.

#### `StalwartInstance` (cluster) et `StalwartLocalInstance` (namespacé)

Ce sont les specs du template, avec les champs que forgejo y ajoute.

| Champ | Type (kit) | Défaut | Rôle |
| --- | --- | --- | --- |
| `url` | string | — | Base HTTP de Stalwart. La gestion passe par `POST <url>/jmap` (session `GET <url>/jmap/session`). `/api` ne sert qu'à `schema` (D13). |
| `tokenSecretRef` | `SecretKeyRef` (locale : `LocalSecretKeyRef` + label `instance-token`) | — | ApiKey, relue à chaque appel |
| `sharedWith` | `SharedWith` | — | locale seulement, comme le template |
| `tls` | `TlsOptions` (locale : `LocalTlsOptions`) | — | `caSecretRef`, `insecureSkipVerify` |
| `secretStore` | `{backend: SecretStoreBackend, vault: {…, pathGrants: []VaultPathGrant}}` | `kubernetes`, `pathPrefix: weebo-stalwart` | backend par défaut des `secretTargets` |
| `submission` | `{host, port, security}` | `stalwart.mail.svc.cluster.local`, `587`, `starttls` | Spécifique à Stalwart : ce que les credentials publiés annoncent |

Les hashes de schéma supportés sont une constante de l'opérateur, comme la
plage de `UnsupportedForgejoVersion`, et pas un champ. `ping` lit
l'en-tête `Location` de `GET /api/schema` (sans suivre la redirection) et
refuse un hash inconnu (D13).

#### `StalwartNamespacePolicy` (cluster)

La forme de `ForgejoNamespacePolicy`, évaluée par le moteur `allow_list`
du kit. La dimension `names` porte le nom **et** les alias : chaque namespace
d'app liste les siens, et une règle `Deny` réserve les noms d'infra pour tous
les kinds de compte :

```yaml
apiVersion: stalwart.weebo.io/v1alpha1
kind: StalwartNamespacePolicy
metadata:
  name: weebo-allow
spec:
  rules:
    - namespaces: [mail]
      allowedKinds: [StalwartAccount, StalwartGroup, StalwartMailingList]
      allowedDomains: [weebo.poc]        # dimension "domains"
    - namespaces: [forgejo]
      allowedKinds: [StalwartServiceAccount]
      allowedDomains: [weebo.poc]
      allowedNames: [forgejo, noreply]   # dimension "names" : nom et alias
    - namespaces: [auth]
      allowedKinds: [StalwartServiceAccount]
      allowedDomains: [weebo.poc]
      allowedNames: [authentik]
    - namespaces: [argocd]
      allowedKinds: [StalwartServiceAccount]
      allowedDomains: [weebo.poc]
      allowedNames: [argocd]
    - effect: Deny                        # noms réservés, partout
      namespaces: ["*"]
      allowedKinds: [StalwartServiceAccount, StalwartAccount, StalwartGroup,
                     StalwartMailingList]
      allowedNames: [postmaster, abuse, admin, root, weebo-stalwart]
```

Un `Deny` gagne toujours, et il touche une requête dont un nom ou un alias
est listé (`allow_list::evaluate`). `noreply` n'y est donc pas : il est
réservé à forgejo par son `Allow`, puis par la revendication unique. Le
namespace `mail` ne borne pas les noms, puisque la brique de liaison y crée
un compte par utilisateur ; il est sous le seul contrôle de Git et des règles
de weebo-link.

Seuls les kinds namespacés liés à une instance cluster sont gouvernés.
`StalwartDomain`, les instances et la policy restent sous RBAC (Argo CD),
comme chez forgejo.

#### `StalwartDomain` (cluster) → `x:Domain`

| Champ | Type | Défaut | Champ Stalwart |
| --- | --- | --- | --- |
| `instanceRef` | `InstanceRef` | l'unique instance | — |
| `name` | DomainName | `metadata.name` | `name` |
| `aliases` | []DomainName | `[]` | `aliases` (`Set<DomainName>`) |
| `catchAll` | EmailLocalPart? | — | `catchAllAddress` (`EmailAddress?`) : `<catchAll>@<name>` |
| `subAddressing` | `Enabled` \| `Disabled` | `Enabled` | `subAddressing` (la variante `Custom` n'est pas exposée) |
| `dkim` | `Automatic` \| `Manual` | `Automatic` | `dkimManagement` ; `Automatic` avec les paramètres par défaut du serveur |
| `deletionPolicy` | `Orphan` | `Orphan` | — (D18) |

L'opérateur pose aussi `certificateManagement: Manual`,
`dnsManagement: Manual` et `allowRelaying: false`, qui ne sont pas exposés.
`dkimManagement: Automatic` a des paramètres (`algorithms`, Ed25519 et RSA
par défaut, `selectorTemplate`, `rotateAfter`, `retireAfter`,
`deleteAfter`). Sans `dnsManagement` automatique, les clés sont générées et
actives, sans rotation ni publication (`services/src/task_manager/dkim.rs`) ;
c'est suffisant pour un domaine interne. Changer `name` est une mise à jour
disruptive : il faut `allow-disruptive-update: "true"`.

#### `StalwartServiceAccount` (namespacé) → `x:Account` `@type: User` + credential `Password`

| Champ | Type | Défaut | Rôle |
| --- | --- | --- | --- |
| `instanceRef` | `InstanceRef` | — | |
| `domainRef` | `ObjectRef` (cluster-scoped) | — | le `StalwartDomain` |
| `name` | EmailLocalPart | `metadata.name` | `name` (dimension `names`) |
| `aliases` | []EmailLocalPart | `[]` | `aliases` : `{name, domainId, enabled: true}`, dans le domaine du compte, vérifiés aussi par `names` |
| `receive` | bool | `false` | `false` : `EmailReceive` dans `disabledPermissions` |
| `secretTargets` | []`SecretTarget` | cible implicite du kit (backend `kubernetes` seulement) | clés `host`, `port`, `security`, `username`, `password`, `from` ; une cible Vault s'appelle `smtp` (D15) |
| `deletionPolicy` | `Delete` \| `Orphan` | `Delete` | |

À la création, l'opérateur pose les champs que `x:Account` exige :
`roles: User`, `encryptionAtRest: Disabled`, et `permissions` en `Merge`
avec `disabledPermissions` : `ImapAuthenticate`, `Pop3Authenticate`,
`SieveAuthenticate`, plus `EmailReceive` si `receive: false`. Il génère le
mot de passe, le pose comme credential `Password` et l'écrit par
`emit::write_all`. Il le régénère sur un nouveau nonce `rotate-secret`
(`emit::rotation_requested` contre `status.lastRotation`), ou quand
`emit::any_missing` détecte une cible qui l'a perdu. Une seule cible Vault
par namespace : deux comptes d'envoi du même namespace visent le même
`mv/<ns>/smtp`, et l'owner refuse le second (il publie alors en
`kubernetes`).

#### `StalwartAccount` (namespacé) → `x:Account` `@type: User`

| Champ | Type | Défaut | Rôle |
| --- | --- | --- | --- |
| `instanceRef` | `InstanceRef` | — | |
| `domainRef` | `ObjectRef` (cluster-scoped) | — | |
| `name` | EmailLocalPart | `metadata.name` | doit égaler le `preferred_username` Authentik |
| `aliases` | []EmailLocalPart | `[]` | `aliases`, comme ci-dessus |
| `quota` | Quantity? | — | `quotas.maxDiskQuota`, en octets |
| `locale`, `timeZone` | string? | — | |
| `state` | `Active` \| `Locked` | `Active` | voir *Verrouillage* |
| `deletionPolicy` | `Delete` \| `Orphan` | `Orphan` | une boîte humaine ne se supprime qu'explicitement |

Création avec `roles: User`, `encryptionAtRest: Disabled`,
`permissions: Inherit` (ou `Merge` si `Locked`). Pas de credential :
l'authentification passe par OIDC, et les App Passwords par la WebUI (D14).
Ni `description` ni `memberGroupIds` (D5, D6).

#### `StalwartGroup` (namespacé) → `x:Account` `@type: Group`

Champs `instanceRef`, `domainRef`, `name`, `aliases`, `quota` et
`deletionPolicy` (`Orphan` par défaut). Création avec `roles: Default`.
`name` égale le nom du groupe Authentik publié dans le claim `groups`.

La JIT crée elle-même un `x:Account` `Group` pour chaque groupe du claim,
au premier login d'un membre (`synchronize_group`). Un `StalwartGroup`
posé après échoue donc toujours en `StalwartObjectAlreadyExists`. Il doit
exister **avant** que le groupe soit publié dans le claim ; sinon il passe
par l'importer, comme un compte JIT (D9). Un groupe sans CR fonctionne
quand même : seuls son quota et ses alias ne sont pas gérés.

#### `StalwartMailingList` (namespacé) → `x:MailingList`

Champs `instanceRef`, `domainRef`, `name`, `aliases`, `deletionPolicy` et
`recipients`. `recipients` est une liste de `{address}` ou de `{accountRef}`,
où `accountRef` est un `ObjectRef` vers un `StalwartAccount` du même
namespace, résolu en son `status.address`. Côté Stalwart, `recipients` est
un ensemble d'adresses (`Set<EmailAddress>`).

#### Champs comparés et normalisation

Règle de la RFC 0003 : on compare à la forme que le serveur stocke, un champ
`None` dans la spec est « pas d'avis », une liste sans ordre se compare
comme un ensemble, et chaque normalisation a son test de contrat.

| Kind | Champ | Comparaison | Attention |
| --- | --- | --- | --- |
| tous | `emailAddress` | jamais écrit | posé par le serveur (`name@domaine`) ; lu pour `status.address` et pour D19 |
| Domain | `name`, `catchAllAddress`, `allowRelaying` | égalité | `catchAllAddress` comparé à l'adresse complète |
| Domain | `aliases` | ensemble | |
| Domain | `subAddressing`, `certificateManagement`, `dnsManagement` | variante (`@type`) | |
| Domain | `dkimManagement` | variante seule | paramètres de `Automatic` = défauts du serveur, pas d'avis |
| ServiceAccount, Account, Group | `aliases` | ensemble de `(name, domainId)`, `enabled: true` | `description` d'alias ignorée. La JIT OIDC n'ajoute aucun alias (`build_account` passe `email_aliases` vide) ; une directory LDAP ou SQL en ajouterait (`synchronize_account`), et il faudrait alors comparer en sous-ensemble |
| ServiceAccount, Account | `roles` | variante, `User` | un compte passé `Admin` à la main est remis en `User`, avec un Event |
| Group | `roles` | variante, `Default` | |
| tous les comptes | `encryptionAtRest` | non comparé | posé `Disabled` à la création ; un choix fait ensuite dans la WebUI est gardé |
| ServiceAccount, Account | `permissions` | sous-ensemble : les seules permissions que l'opérateur pose, présentes ou absentes de `disabledPermissions` | les autres entrées sont laissées ; `Inherit` distant = aucune désactivation |
| Account, Group | `quotas` | sous-ensemble de la map, clé `maxDiskQuota` | `2Gi` (Quantity) converti en octets (`2147483648`) avant comparaison ; les autres clés sont des défauts du serveur |
| Account | `locale`, `timeZone` | égalité si renseignés | `locale` vaut `en-US` par défaut côté serveur |
| Account | `description`, `memberGroupIds` | non comparés | écrits par la JIT (D5, D6) |
| MailingList | `recipients` | ensemble d'adresses | `accountRef` résolu avant comparaison |
| ServiceAccount | credential `Password` | jamais relu | écrit à la création et à la rotation seulement |

Le test « reste silencieux » du template (deux réconciliations, aucune
écriture au second passage) tourne **contre l'image Stalwart réelle**, pas
seulement contre un faux distant : c'est la règle 3 de la RFC 0003, après
la dérive permanente des `expression` de weebo-authentik que wiremock
n'avait pas vue.

#### Ce que la brique de liaison consomme

L'opérateur n'a aucun kind « source » : la brique de liaison écrit des
`StalwartAccount` (et au besoin des `StalwartGroup`) comme le ferait un
humain dans Git. Ce que cette RFC lui garantit, et ce qu'elle attend d'elle :

- **Écriture :** des `StalwartAccount` dans un namespace autorisé par la
  `StalwartNamespacePolicy` (`mail` dans weebo-si). Elle passe par le
  webhook et l'allow-list comme tout client ; elle n'a aucun droit sur
  Stalwart lui-même.
- **Identité :** `spec.name` égale le `preferred_username` Authentik. La JIT
  retombe alors sur le même compte : elle cherche par local-part et domaine
  (`account_id_from_parts`). Changer `spec.name` est une mise à jour
  disruptive : la brique crée un nouveau CR plutôt que de renommer.
- **Départ :** `spec.state: Locked`, pas une suppression (D8). Une
  suppression applique la `deletionPolicy` du CR, `Orphan` par défaut.
- **Lecture :** `status.conditions` (`Ready`, raison) et `status.address`,
  pour qu'elle reporte l'état côté source si elle le souhaite.
- **Compte JIT déjà présent :** le CR échoue en
  `StalwartObjectAlreadyExists`. La reprise passe par
  `weebo-stalwart-importer` (`import-pending`, garde `RemoteObjectClaimed`) ;
  l'opérateur n'adopte jamais par nom (D9). Que la brique pose elle-même
  `import-pending` est une question de sa RFC.

La RFC 0002 en tire l'**état sûr** de `StalwartAccount` : `state: Locked`
et `Ready` à jour. **`Locked` appliqué donne `Ready: True`, raison
`Reconciled`** (D20) : c'est ce que son expression `safe` lit
(`spec.state == "Locked"`, `observedGeneration` à jour, `Ready == True`).
Un verrouillage qui échoue garde `Ready: False` avec sa raison, et la cible
reste `Detaching`. Au départ d'un utilisateur lié, weebo-link patche la
cible vers cet état et la garde (`Detached`) au lieu de la supprimer.

Tant que la brique n'existe pas, ces CR se déclarent à la main dans
`weebo-si:2.argo/helm/mail`.

#### Verrouillage

Le schéma `Account` de v0.16 n'a pas de champ `disabled`. `state: Locked`
fait deux choses :

1. ajoute `Authenticate`, `EmailSend` et `EmailReceive` aux
   `disabledPermissions` (`permissions: Merge`). C'est le mécanisme du SCIM
   de Stalwart (`scim/src/users/mod.rs`, `set_active`, qui désactive
   `Authenticate`). Toute authentification passe par
   `assert_has_permission(Authenticate)` (`common/src/auth/authentication.rs`),
   App Passwords compris : ils sont **gardés**, et inutilisables tant que le
   compte est verrouillé ;
2. garde les mails, l'adresse et les alias, pour qu'aucun autre CR ne
   puisse les reprendre : la revendication de `remoteId` reste.

`Active` retire les seules désactivations posées par l'opérateur. Les App
Passwords de l'utilisateur refonctionnent alors : les détruire au
verrouillage serait redondant et irréversible.

#### Codes de raison (`reason_codes!`)

Ceux du template v0.18, inchangés : `Reconciled`, `InstanceNotFound`,
`InstanceAmbiguous`, `InstanceNotAllowed`, `InstanceBindingChanged`,
`TokenSecretMissing`, `TokenSecretNotAllowed`, `InvalidSpec`,
`InstanceUnreachable`, `InstanceUnauthorized`, `RemoteError`,
`KubernetesApiError`, `Created`, `DriftCorrected`, `Deleted`, `Released`
et `LeavesWatchScope`.

Ceux qui reprennent le nom déjà utilisé par forgejo : `NamespaceNotAllowed`,
`SecretTargetFailed`, `RemoteObjectClaimed`,
`StalwartObjectAlreadyExists` (comme `ForgejoObjectAlreadyExists`) et
`UnsupportedStalwartVersion` (hash de schéma inconnu, D13).

Ceux qui sont propres à Stalwart : `DomainNotFound`, `DomainNotReady`,
`AccountNotFound` (un `accountRef` de liste qui ne résout pas) et
`ObjectIsLinked` (suppression refusée par le serveur).

`Created`, `DriftCorrected`, `Deleted`, `Released` et `Reconciled` sont des
avis (`Severity::Advisory`). Tout le reste bloque.

#### Webhook

Ce sont l'`Admission` et le `router` du runtime, en `failurePolicy: Fail` sur
les seuls kinds `stalwart.weebo.io`. Contrôles :

- l'allow-list (`allow_list::evaluate`, dimensions `domains` et `names`) ;
- `check_vault_targets` ;
- le `ScopeGuard` (`LeavesWatchScope`) ;
- les alias restent dans le domaine du compte ;
- une cible Vault d'un `StalwartServiceAccount` s'appelle `smtp`, sans
  `path`, et la cible implicite n'est admise qu'en backend `kubernetes`
  (D15) ;
- `deletionPolicy: Delete` refusé sur `StalwartDomain` (D18).

Le reconciler refait les mêmes contrôles (`InvalidSpec`). L'unicité
`domainRef + name` entre CR n'est pas vérifiée à l'admission : elle l'est au
reconcile, par la revendication unique (D16).

#### Métriques

`ReconcileMetrics` du kit, avec le préfixe `weebo_stalwart`, dont
`weebo_stalwart_remote_writes_total{kind, cause}` (`created`, `drift`,
`deleted`). En plus :

- `weebo_stalwart_accounts{state}` ;
- `weebo_stalwart_api_requests_total{method,outcome}`.

La page de référence des métriques et des codes vient de `@weebo/operator-docs`.

### Architecture

La réponse est **oui, hexagonale**, avec les crates du template, renommées
par `cargo generate`, plus l'`importer` de forgejo :

| Crate | Contenu propre à Stalwart |
| --- | --- |
| `domain` | `ReasonCode` ; règles pures (adresse, alias dans le domaine, permissions désactivées pour un état, conversion de quota) ; comparaisons avec `weebo-kit-domain::drift` |
| `api` | les CRD, sur les types de `weebo-kit-api` ; `Status` avec `address` et `lastRotation` (repris de forgejo) |
| `application` | `RemoteGateway` étendu par kind (`ping`, `get_*` par id, `create_*`, `update_*`, `delete_*`), `GatewayFactory` ; use-cases `reconcile_instance`, `reconcile_domain`, `reconcile_service_account`, `reconcile_account`, `reconcile_group`, `reconcile_mailing_list` ; `emit` comme chez forgejo |
| `adapters-outbound` | `JmapGateway` (à la place de `http_gateway.rs`), sur `POST /jmap` ; `gateway_factory.rs` du template (jeton relu à chaque appel) avec la `SecretStoreFactory` du kit, comme forgejo ; `ClusterReader::claimants` repris de forgejo (D16) |
| `adapters-inbound` | controllers et webhook du template, rien hors de `stalwart.weebo.io` |
| `importer` | `weebo-stalwart-importer`, sur le modèle de `crates/importer` de forgejo (absent du template) : relie un objet Stalwart existant à un CR (`import-pending`) |
| `crdgen`, `operator`, `testkit` | ceux du template ; le testkit ajoute un Stalwart v0.16.27 en conteneur pour les tests de contrat |

**Réconciliation d'un compte**, sur le modèle de `reconcile_object` du
template v0.18, à la lecture près (D9) :

1. Résoudre l'instance (`InstanceDirectory`, `status.instance`) et le
   domaine. Si le domaine n'est pas `Ready`, `DomainNotReady`.
2. Évaluer la policy (même verdict que le webhook).
3. Lire. Avec `status.remoteId` : `x:Account/get` par id. Une réponse vide,
   ou dont l'adresse diffère de `status.address` (D19), vaut objet absent.
4. Absent : vérifier qu'aucun autre CR ne revendique `domainRef + name`
   (`claimants`), puis `x:Account/set create`. Un nom déjà pris côté
   Stalwart donne `StalwartObjectAlreadyExists` : on n'adopte jamais.
   Écriture comptée `cause="created"`.
5. Présent : construire l'état voulu, le comparer aux seuls champs possédés
   (*Champs comparés*), et n'envoyer un `x:Account/set update` que des
   champs qui diffèrent. Une dérive émet un Event `DriftCorrected` qui les
   nomme et compte `cause="drift"`. Sans dérive, aucun appel en écriture.
6. Secrets : `emit::check_targets`, puis `write_all` à la création, sur un
   nouveau nonce de rotation ou si `any_missing`.
7. Status (`address`, `lastRotation`), condition, Event.

La suppression suit le finalizer du runtime : `force-orphan` libère sans
rien toucher, `deletionPolicy: Orphan` libère en laissant l'objet distant,
et `Delete` détruit l'objet distant et appelle `emit::delete_all`.
`objectIsLinked` donne `ObjectIsLinked`, et l'opérateur requeue sans forcer.

### Données et état

L'opérateur est **sans état**. La vérité est dans les CR, dans Stalwart et
dans le `SecretStore`.

- **`status.remoteId` perdu** (status effacé) : le CR retombe en création
  et échoue en `StalwartObjectAlreadyExists`. La reprise passe par
  l'importer, comme chez forgejo. Une migration qui déplace un CR pose
  `previous-owner` pour qu'il retrouve ses secrets.
- **Mots de passe des comptes d'envoi :** écrits par `emit`, jamais relus
  côté Stalwart. Si la cible les perd, `any_missing` déclenche une rotation.
- **Stalwart repart d'une base vide :** les `remoteId` ne résolvent plus,
  ou résolvent vers un autre compte si les ids sont réattribués ; D19 les
  traite tous deux comme absents, et le CR est recréé côté Stalwart. Les
  mails sont perdus, les mots de passe sont régénérés et republiés.

### Déploiement de Stalwart dans weebo-si

- **Image :** `stalwartlabs/stalwart:v0.16.27`, épinglée et montée par
  Renovate. Les patchs sortent chaque semaine (0.16.25 le 2026-10-05,
  0.16.26 le 2026-10-11, 0.16.27 le 2026-10-18). Le namespace `mail` est
  hors Che, mais on tire quand même par angos si `imagePolicy` est étendu un
  jour (Q7).
- **`config.json`** (Secret) ne contient que le DataStore :

  ```json
  { "@type": "PostgreSql", "host": "stalwart-rw.mail.svc", "port": 5432,
    "database": "stalwart", "authUsername": "stalwart",
    "authSecret": { "@type": "EnvironmentVariable", "variableName": "STALWART_DB_PASSWORD" } }
  ```

- **BlobStore :** un singleton (id `singleton`, `update` seulement), posé
  par le Job de bootstrap. La région est une variante imbriquée :

  ```json
  { "@type": "S3", "bucket": "mail",
    "region": { "@type": "Custom", "customEndpoint": "https://s3-main.weebo.poc",
                "customRegion": "weebo" },
    "accessKey": …, "secretKey": … }
  ```

  Les credentials (`mv/mail/s3`) sont dans `weebo-si:2.terra/vault/terra-map/s3.tf`,
  à côté de `forgejo/s3`. Le bucket, son user et sa policy viennent d'une
  entrée `- name: mail` dans `s3.mainInstance.creds` de
  `weebo-si:2.argo/helm/database/sub/values.yaml`, comme ceux de forgejo
  (`weebo-si:2.argo/helm/forgejo/values.yaml`, `[storage]`). Le nom de
  l'entrée donne le user, le bucket et le chemin `mv/<nom>/s3`.
- **Listeners :** submission 587 (STARTTLS), IMAPS 993, HTTPS 443 (WebUI
  et JMAP), management 8080 en interne. Tous en ClusterIP. Pas de SMTP 25
  (D22). Seul le HTTPS est exposé, par une IngressRoute Traefik
  `mail.weebo.poc`.
- **TLS :** Certificate cert-manager (CA interne) monté en volume. Un
  `x:Certificate` accepte la variante `File` pour `certificate` et
  `privateKey` (`PublicText`, `SecretText`) : il pointe les fichiers montés,
  sans recopie du PEM. Reste à vérifier que Stalwart relit les fichiers au
  renouvellement (Q5).
- **Probes :** `/healthz/live` et `/healthz/ready` sur le listener de
  management.
- **CiliumNetworkPolicy :**
  - egress limité à kube-dns, `stalwart-rw` (5432) et Traefik (443) :
    `auth.weebo.poc` (issuer, discovery, JWKS) et `s3-main.weebo.poc`
    résolvent vers Traefik dans le cluster, comme pour les autres clients
    OIDC (`weebo-si:2.argo/helm/ingress/sub/templates/middleware/network-policy.yaml`).
    Si la policy `traefik-ingress`, aujourd'hui désactivée
    (`fromWholeCluster`), est restreinte un jour, `mail` doit y figurer ;
  - ingress 587/993 limité aux namespaces listés dans les values, 443
    depuis Traefik ;
  - 8080 depuis `weebo-stalwart` (l'opérateur) et depuis `mail` (le Job).
- **Authentik :** une entrée `stalwart` dans `weebo-si:2.argo/helm/auth/sub/values.yaml`
  (`applications`), provider oauth2, client public pour la WebUI, audience
  `stalwart`, scopes `openid email profile groups`, `secretTargets` vers
  `mv/mail/auth`. La directory `Oidc` de Stalwart :

  ```json
  { "@type": "Oidc", "issuerUrl": "https://auth.weebo.poc/application/o/stalwart/",
    "requireAudience": "stalwart", "claimUsername": "preferred_username",
    "usernameDomain": "weebo.poc", "claimName": "name", "claimGroups": "groups" }
  ```

  Un `preferred_username` sans `@` devient `<valeur>@weebo.poc`
  (`resolve_email`), et chaque groupe du claim `<groupe>@weebo.poc`.
- **Bootstrap** (D17), sur le modèle de
  `weebo-si:2.argo/helm/forgejo/templates/operator/token.yaml` : un même pod
  lancé en Job PostSync `stalwart-bootstrap` (après chaque sync) et en
  CronJob quotidien (ce qui arrive entre deux syncs : clé révoquée, Secret
  supprimé, base restaurée). Chaque passage :
  1. attend `/healthz/ready` ;
  2. avec l'admin de recovery (`STALWART_RECOVERY_ADMIN`, lu dans
     `mv/mail/admin`), lance `stalwart-cli apply` sur un plan d'`upsert`
     (BlobStore, Directory `Oidc`, `Authentication.directoryId`, compte
     `weebo-stalwart`) ;
  3. lit le jeton dans le Secret `stalwart-operator-token` (namespace
     `mail`, clé `token`) et s'arrête là si Stalwart l'accepte
     (`GET /jmap/session` en Bearer) : le cas courant ;
  4. sinon, détruit toute ApiKey du même nom sur le compte `weebo-stalwart`
     (un passage interrompu, un Secret supprimé à la main), crée l'ApiKey
     `weebo-stalwart` (permissions `Replace`, voir *Sécurité*), dont
     `x:ApiKey/set` renvoie le secret une seule fois dans `created`, et
     l'écrit dans le Secret.

  La `StalwartInstance` pose `tokenSecretRef: {name: stalwart-operator-token,
  namespace: mail, key: token}`, comme la `ForgejoInstance` lit son jeton
  dans le namespace `forgejo`. Le ServiceAccount du Job n'a que `create`
  sur les Secrets et `get`/`update` sur ce Secret-là, dans `mail`. Il n'a
  aucun droit Vault en écriture : `mail-read` lui suffit. Le Secret est créé
  à l'exécution, hors de l'Application : Argo ne le diffe ni ne le prune.
  L'admin de recovery reste en place comme break-glass, son mot de passe
  dans `mv/mail/admin`, comme `mv/forgejo/admin`.
- **Vault :**
  - `weebo-si:2.terra/vault/terra-map/mail.tf` crée `mv/mail/{pg,admin}`
    (`s3` est dans `s3.tf`, `auth` vient de weebo-authentik), le rôle
    `mail-read` (ExternalSecrets de `mail`) et le rôle `stalwart-operator`
    (SA `weebo-stalwart` du namespace `weebo-stalwart`) ;
  - la `StalwartInstance` pose `secretStore.vault.pathPrefix: ""`. La règle
    du kit (`<pathPrefix>/<namespace>/`) donne alors `mv/<namespace>/smtp`,
    soit `mv/forgejo/smtp` pour un CR du namespace `forgejo`. Aucun
    `pathGrants` n'est nécessaire ;
  - la policy du rôle `stalwart-operator` (D15) :

    ```hcl
    path "mv/data/+/smtp"     { capabilities = ["create", "read", "update", "delete"] }
    path "mv/metadata/+/smtp" { capabilities = ["create", "read", "update", "delete"] }
    ```

    `mv/metadata/…` est indispensable : sans lui, le kit n'enregistre ni ne
    vérifie l'owner, avec un simple warning
    (`crates/weebo-kit-adapters/src/secret_vault.rs`, `metadata` et
    l'enregistrement de l'owner), et le check-and-set avec owner disparaît
    en silence. `create`/`update` sur `metadata` servent à enregistrer
    l'owner, `read` à le vérifier ;
  - **invariant :** avec `pathPrefix: ""`, un chemin sans owner est adopté
    (`RoutingSecretStore::vault_owned`, `secret_routing.rs` ; `docs/plan.md`,
    v0.15.2 : « chemin libre ou sans owner (adopté) »). Tout ce que
    Terraform écrit est sans owner. **Seule la policy `mv/+/smtp`** empêche
    un `StalwartServiceAccount` du namespace `forgejo` dont la cible
    s'appellerait `admin` de réécrire `mv/forgejo/admin`, ou un CR de `mail`
    de réécrire `mv/mail/admin` (le break-glass). Le contrôle d'admission de
    D15 donne un message plus clair, il ne remplace pas la policy. Élargir
    cette policy est un changement de sécurité, à faire par RFC.

### Brancher les apps

| App | Où | Config, alimentée par `mv/<app>/smtp` |
| --- | --- | --- |
| Authentik | `weebo-si:2.argo/helm/auth/main` | `AUTHENTIK_EMAIL__HOST`, `__PORT`, `__USERNAME`, `__PASSWORD`, `__USE_TLS=true`, `__FROM` |
| Forgejo | `weebo-si:2.argo/helm/forgejo` | `[mailer] ENABLED=true`, `PROTOCOL=smtp+starttls`, `SMTP_ADDR`, `SMTP_PORT`, `USER`, `PASSWD`, `FROM` |
| Argo CD | `weebo-si:1.pulu-conf/main.go` (valeurs `notifications` du chart argo-cd, qui rendent `argocd-notifications-cm`) | `service.email: {host, port, from, username: $email-username, password: $email-password}` ; le Secret `argocd-notifications-secret` alimenté depuis `mv/argocd/smtp` |
| Monitoring | ClickStack | alertes email si HyperDX les propose ; sinon hors périmètre |
| BatleHub | `weebo-si:2.argo/helm/batlehub` | quand BatleHub exposera une config SMTP |
| Che | — | rien côté serveur ; un dev peut soumettre sur 587 avec un App Password créé dans la WebUI |

## Sécurité

- **Privilèges dans Stalwart :** l'ApiKey de l'opérateur est en
  `permissions: Replace` avec seulement :
  - `sysDomain{Get,Query,Create,Update}` (pas `sysDomainDestroy`, D18) ;
  - `sysAccount{Get,Query,Create,Update,Destroy}` ;
  - `sysMailingList{Get,Query,Create,Update,Destroy}` ;
  - ce qu'exigent la session JMAP et `/api/schema` (Q1).

  Pas d'`impersonate` (D14), aucun `sysApiKey*` ni `sysAppPassword*`, aucun
  `jmapEmail*`, aucun singleton de settings. `sysApiKey*` existe : seul le
  bootstrap s'en sert, avec l'admin de recovery. Le bootstrap seul touche
  aux singletons.
- **Privilèges Kubernetes :**
  - CRUD sur ses CRDs, et aucun droit sur les CRD d'un autre opérateur ;
  - lecture du Secret `stalwart-operator-token` dans `mail` (jeton de
    l'instance), Secrets des cibles `kubernetes` ;
  - Events et Lease.
- **Vault :** le rôle `stalwart-operator`, sur `mv/data/+/smtp` et
  `mv/metadata/+/smtp` seulement (D15, et l'invariant de *Vault*). La règle
  de chemin du kit interdit à un CR d'écrire hors de son namespace, et le
  check-and-set avec owner empêche d'écraser une valeur qu'un autre objet
  possède.
- **Frontière de confiance :**
  - **Les CR des namespaces d'apps.** Ils sont bornés par la
    `StalwartNamespacePolicy` (kind, domaine, noms), et les alias restent
    dans le domaine du compte. Chaque namespace d'app a une règle `Allow`
    qui liste ses noms et alias (`forgejo: [forgejo, noreply]`), comme
    `allowedUsers` chez forgejo : il ne peut prendre ni `admin@weebo.poc`
    ni le nom d'un futur utilisateur. La règle `Deny` réserve les noms
    d'infra pour tous les kinds de compte, `mail` compris. La revendication
    unique fait le reste : un nom déjà possédé par un CR ne peut pas être
    repris.
  - **La brique de liaison.** Elle crée et verrouille des `StalwartAccount`,
    bornée par la même `StalwartNamespacePolicy` que les autres clients
    (namespace `mail`, `Deny` sur les noms réservés). Compromise, elle peut
    verrouiller des boîtes ou en créer, jamais lire de mail ni toucher aux
    comptes d'envoi des apps, qui vivent dans d'autres namespaces.
- **Contournements :**
  - CR créés avant le webhook : le reconciler revérifie la policy.
  - Comptes créés à la main dans la WebUI : ils sont invisibles pour
    l'opérateur. Un CR du même nom échoue en `StalwartObjectAlreadyExists`
    jusqu'à un import explicite.
  - Comptes et groupes créés par la JIT OIDC avant leur CR : le CR échoue
    en `StalwartObjectAlreadyExists`, qu'il vienne d'un humain ou de la
    brique de liaison. Seul l'importer les relie, après la garde de
    revendication unique.
  - Usurpation de `From` : `MtaStageAuth.mustMatchSender` vaut `true` par
    défaut, donc le `MAIL FROM` doit appartenir au compte authentifié. Que
    ses alias comptent (`From: noreply@…` pour `forgejo`) se vérifie au
    spike (Q8).
- **Rayon d'impact :** un opérateur compromis peut créer, modifier,
  verrouiller ou supprimer des comptes, des groupes et des listes, poser
  le mot de passe des comptes d'envoi, et lire et écrire les secrets
  `mv/+/smtp`. Il ne peut ni changer la config du serveur, ni créer d'ApiKey
  ou d'App Password, ni rien envoyer hors du cluster (D12). **Sans
  `impersonate`, il ne lit aucun mail par JMAP.** Une réserve, à lever au
  spike (Q1) : si `sysAccountUpdate` permet de poser une credential
  `Password` sur n'importe quel compte, il peut s'authentifier comme un
  utilisateur et lire sa boîte. Si c'est le cas, le rayon d'impact inclut
  la lecture des boîtes, et il faudra le dire. Un Stalwart compromis reste
  enfermé dans le namespace `mail` par l'egress.
- **Secrets :** les mots de passe ne sont jamais logués, ni mis dans le
  status ou les Events. Le status ne porte que `lastRotation` (le nonce
  consommé) et `address`. Le client JMAP masque le header `Authorization`
  dans ses traces OTLP.

## Exploitation

- **Mode d'échec :**
  - *Stalwart :* opérateur arrêté, le mail continue de fonctionner. On ne
    perd que la création et le verrouillage.
  - *Webhook :* `Fail`, sur les seuls kinds `stalwart.weebo.io`. Ne rien
    pouvoir créer pendant une panne de l'opérateur est préférable à un
    compte créé hors policy, et ces CR ne sont sur le chemin d'aucun Pod.
  - *Verrouillage :* il dépend de l'opérateur et de la brique de liaison.
    Si l'un des deux est arrêté, un user retiré d'Authentik ne peut déjà
    plus se connecter par OIDC (Authentik le refuse), mais ses App
    Passwords restent valides jusqu'à leur retour. C'est accepté, et visible
    par l'alerte « opérateur sans leader » du kit, une fois branchée
    (*Observabilité*) ; le status, lui, n'avance plus.
  - *Jeton :* une ApiKey révoquée met l'instance en `InstanceUnauthorized`
    jusqu'au prochain passage du CronJob (au plus un jour), ou jusqu'à un
    resync de l'Application `mail`.
- **Rollout :**
  1. Phase 0 : Stalwart, bootstrap et domaine par un plan `apply`, sans
     opérateur. Envoi de test depuis un pod.
  2. Opérateur avec `StalwartInstance`, puis `weebo-stalwart-importer` pour
     relier le domaine et les comptes créés par le plan à leurs CR
     (`import-pending`). C'est le chemin de migration du kit, jamais une
     adoption par nom. Pour un compte d'envoi, la ressource Terraform de
     `mv/<app>/smtp` est retirée d'abord, puis l'import force une rotation
     (D21).
  3. `StalwartServiceAccount` forgejo, test de reset de mot de passe de bout
     en bout, puis Authentik, puis Argo CD.
  4. `StalwartAccount` d'un seul user déclaré à la main, verrouillage et
     déverrouillage vérifiés (App Password refusé verrouillé, accepté
     déverrouillé). La généralisation à tous les users passe par la brique
     de liaison, dans sa propre RFC.
- **Rollback :**
  - Supprimer un CR avec `force-orphan` laisse l'objet Stalwart intact.
  - Couper l'Application `weebo-stalwart` laisse Stalwart servir avec ce
    qu'il a.
  - Revenir en phase 0, c'est réappliquer le plan.
  - Les apps reviennent à « pas de mail » en retirant leur config SMTP.
- **Observabilité :**
  - `Ready` par CR, Events (`DriftCorrected` compris) et métriques du kit
    vers otel-cluster ;
  - métriques Stalwart scrapées ;
  - santé des pods : les probes kubelet `/healthz/*` remontent déjà dans
    « Probe health », qui agrège les Events kubelet `Unhealthy`
    (`weebo-si:2.argo/helm/monitoring/dashboards/README.md`). Une sonde
    TCP+STARTTLS sur 587 et TLS sur 993 suppose un blackbox exporter, que
    `weebo-si:2.argo/helm/monitoring` n'a pas : brique à créer (D22) ;
  - alertes souhaitées : `StalwartInstance` pas `Ready` depuis plus de
    10 min, file sortante non vide depuis plus de 15 min (rien ne devrait y
    rester puisque tout est local), un `StalwartAccount` pas `Ready` depuis
    plus de 10 min, et `RepeatedDrift` du kit. Le `PrometheusRule` du chart
    `weebo-operator-lib` suppose un Prometheus, et weebo-si n'a aujourd'hui
    aucun moteur d'alerte (otel vers ClickStack, sans alerte provisionnée).
    Le mécanisme est à choisir (Q11).
- **Montée de version :**
  - **Stalwart.** Image épinglée au patch. Un nouveau patch passe d'abord
    par le test de contrat du client contre l'image (dont « reste
    silencieux »), puis par la liste des hashes de schéma de l'opérateur
    (D13). Un patch qui change le schéma demande donc une release de
    l'opérateur avant la PR Renovate de weebo-si ; un patch qui ne le change
    pas passe sans release (Q10).
  - **Opérateur.** Une seule réplique active (lease), et les objets sont
    idempotents. Pendant un rolling update, l'ancien et le nouveau ne
    réconcilient jamais en même temps.

## Alternatives considérées

1. **Pas d'opérateur : un Job `stalwart-cli apply` rendu depuis les values
   Helm.** Les plans d'`upsert` sont idempotents, `reconcile` sait même
   supprimer ce qui manque. C'est suffisant pour les objets statiques
   (domaine, comptes des apps, groupes), et c'est exactement la **phase 0**.
   Ça ne couvre pas :
   - l'écriture des mots de passe vers Vault (il faudrait les générer dans
     Terraform et les injecter dans le plan, soit deux sources de vérité) ;
   - le verrouillage ;
   - un état par objet.

   L'opérateur ne remplace le plan que là où le plan ne suffit pas.
2. **SCIM depuis Authentik.** C'est la réponse officielle aux points 1 et 2
   de la motivation, et Authentik sait émettre du SCIM. Mais c'est l'édition
   Enterprise de Stalwart, donc une licence, et ça ne fait rien pour les
   comptes d'apps ou Vault. À reconsidérer si une licence arrive.
3. **Provider Terraform dans `weebo-si:2.terra`.** C'est cohérent avec
   `weebo-si:2.terra/auth`, et des providers communautaires existent pour
   l'API de la v0.16 (`flungo/terraform-provider-stalwart`, sur JMAP ;
   `tahacodes/terraform-provider-stalwart`, généré du schéma). Mais
   Terraform ne tourne qu'à l'`apply` : pas de boucle de réconciliation,
   donc pas de dérive corrigée ni d'état par objet. Le cycle de vie des
   users vit dans Argo et dans la brique de liaison, pas dans Terraform. Et
   on dépendrait d'un provider tiers sur une API qui bouge chaque semaine.
4. **Étendre weebo-authentik** pour qu'il pousse lui-même les comptes vers
   Stalwart. Ça mélangerait deux produits dans un opérateur, et ça casse la
   règle du kit : un opérateur par produit.
5. **Un kind `StalwartAccountSource` dans weebo-stalwart**, qui watcherait
   les `AuthentikUser` (brouillon initial de cette RFC). Même défaut, dans
   l'autre sens : l'opérateur Stalwart dépendrait du CRD d'un autre
   opérateur, et chaque paire de produits réinventerait son propre lien.
   Écarté au profit d'une brique de liaison commune (D7).
6. **Un autre MTA.**
   - Mailpit capture les mails des apps sans rien délivrer, ce qui suffit
     pour du dev mais ne donne de boîte à personne.
   - Postfix + Dovecot + LDAP coûtent trois briques à opérer, et
     l'authentification OIDC y est bricolée.
   - Maddy est plus léger, mais sans OIDC ni API de gestion.
7. **Ne rien faire.** Les apps restent sans mail : pas de reset de mot de
   passe autonome, pas de notifications. C'est vivable aujourd'hui (un seul
   utilisateur réel), mais pas avec les équipes que prépare
   weebo-si-hardening.
8. **`StalwartAppPassword` avec `impersonate`** (version précédente de
   cette RFC). Ça donnerait des App Passwords déclaratifs, publiés dans
   Vault, mais l'opérateur pourrait lire toutes les boîtes, et il faudrait
   décider qui lit le secret et comment il arrive à l'utilisateur. Écarté
   pour la v1 (D14).

## Inconvénients et risques

- **Un quatrième opérateur maison à maintenir**, sur une API amont jeune.
  La v0.16 vient de remplacer toute l'API REST, et les patchs sortent
  chaque semaine. On l'atténue par le test de contrat contre l'image et par
  le contrôle du hash de schéma, au prix d'une release de l'opérateur pour
  chaque schéma nouveau.
- **Deux écrivains sur `x:Account`** (opérateur et JIT OIDC). D5 et D6 les
  séparent par champ, et le source les liste : `description`,
  `memberGroupIds`, comptes `Group` créés, aucun alias en OIDC. Si une
  version future de la JIT écrit un champ qu'on possède, l'objet oscille :
  `DriftCorrected` et `remote_writes_total{cause="drift"}` le rendent
  visible tout de suite.
- **Le verrouillage n'est pas un état natif.** Il repose sur des permissions
  désactivées : si l'opérateur part, l'état `Locked` reste dans Stalwart,
  mais personne ne le surveille.
- **Pas d'App Password déclaratif.** Un utilisateur doit créer le sien dans
  la WebUI, et l'opérateur ne les voit pas.
- **Pas de garantie de délivrance hors du cluster.** C'est voulu, mais une
  app configurée pour écrire à une adresse externe verra ses mails rejetés.
  Ce sera visible dans la file Stalwart, pas dans l'app.
- **Les boîtes des users dépendent d'une brique qui n'existe pas encore.**
  Tant que la brique de liaison n'est pas là, les `StalwartAccount` se
  déclarent à la main, et un départ d'Authentik ne verrouille rien tout
  seul. La v1 de l'opérateur reste utile sans elle (comptes d'envoi des
  apps), et son contrat CR ne bouge pas quand elle arrive.

## Questions ouvertes

Les questions bloquantes sont tranchées par un **spike** (Stalwart v0.16.27
en podman, plan de test dans le plan d'implémentation) avant de passer en
`Accepted`. Les réponses déjà lues dans le source (`main` `aa47d8e`) sont
notées ; le spike les confirme sur l'image.

| # | Question | Bloquant |
| --- | --- | --- |
| Q1 | Le Bearer est acquis (`stalwart-cli --api-key`, `STALWART_TOKEN`). Reste : avec `Replace`, quelles permissions faut-il en plus des `sys*` pour `GET /jmap/session`, `POST /jmap` et `GET /api/schema` ? Et `sysAccountUpdate` permet-il de poser une credential `Password` sur un compte utilisateur (rayon d'impact) ? Si oui, peut-on l'interdire sans perdre les comptes d'envoi ? | oui |
| Q2 | ~~`x:AppPassword/set` renvoie-t-il `secret` ? Peut-on en créer pour autrui ?~~ **Lu dans le source :** `{id, secret}` dans `created`, une seule fois ; pour un autre compte, il faut `impersonate` (D14). Reste : un utilisateur OIDC peut-il créer son App Password dans la WebUI, et avec quelles permissions (`Inherit` par défaut) ? | non (v1 sans `StalwartAppPassword`) |
| Q3 | ~~La JIT rattache-t-elle le compte pré-créé ? Quels champs écrit-elle ?~~ **Lu dans le source :** elle reprend le compte de même local-part et domaine, réécrit `description` (claim `name`), remplace `memberGroupIds`, crée un `Group` par groupe du claim, n'ajoute aucun alias en OIDC. Reste : un nom de groupe qui n'est pas un local-part valide (`authentik Admins`) fait-il échouer le login, ou est-il ignoré ? | non |
| Q4 | ~~Forme de `Account.permissions` ?~~ **Lu dans le source :** `Merge` avec `disabledPermissions`, qui priment ; `EmailSend` vérifié à l'AUTH SMTP, `EmailReceive` à la livraison. Reste : un destinataire verrouillé est-il refusé au `RCPT` ou accepté puis rejeté par un DSN ? Un App Password en permissions `Replace` contourne-t-il le `Authenticate` désactivé du compte ? | non |
| Q5 | ~~Un `x:Certificate` doit-il porter le PEM ?~~ **Tranchée :** variante `File` pour le certificat et la clé. Reste : Stalwart relit-il les fichiers quand cert-manager les renouvelle ? | non (contournement : redémarrage au renouvellement) |
| Q6 | ~~Faut-il `sysDomainDestroy` ?~~ **Tranchée :** non, `StalwartDomain` n'accepte que `Orphan` (D18). | ~~non~~ |
| Q7 | Faut-il faire passer le pull de l'image Stalwart par angos ? | non |
| Q8 | ~~Stalwart refuse-t-il un `MAIL FROM` hors du compte ?~~ **Lu dans le source :** `MtaStageAuth.mustMatchSender` vaut `true` par défaut. Reste : les alias du compte sont-ils acceptés comme expéditeur ? | non |
| Q9 | ~~`dkimManagement: Automatic` sans DNS génère-t-il une clé ?~~ **Tranchée :** oui, clés générées et actives, sans rotation ni publication. | ~~non~~ |
| Q10 | Le hash de `/api/schema` change-t-il à chaque patch, ou seulement quand le schéma change ? Si c'est à chaque patch, D13 coûte une release par semaine : on garderait alors le seul test de contrat, sans contrôle à l'exécution. | non |
| Q11 | Quel moteur d'alerte pour weebo-si : alertes ClickStack (HyperDX), ou un Prometheus pour les `PrometheusRule` du kit ? Commun à tous les opérateurs, donc à trancher hors de cette RFC. | non |

## Travaux futurs

- **Mode *managed* :** `StalwartInstance.spec.managed` déploie Stalwart avec
  le socle workload du kit v0.17, comme weebo-angos le prévoit, et remplace
  le chart maison de `weebo-si:2.argo/helm/mail`.
- **`StalwartAppPassword`**, si un besoin d'App Passwords déclaratifs
  apparaît. Il faudra alors décider ensemble : `impersonate` pour
  l'opérateur, ou une voie amont qui n'en a pas besoin ; un namespace dédié
  aux CR utilisateurs (`mailboxes`), distinct de `mail` qui porte le
  serveur et ses secrets ; un `pathPrefix` non vide pour ces cibles, et une
  policy Vault qui exclut `mv/mail/{admin,pg,s3,auth}` ; la remise du secret
  à l'utilisateur (qui lit le chemin, avec quelle policy).
- **Sonde blackbox** 587/993 et panneau dans « Probe health », et
  **alertes** (Q11).
- **Domaine public `weebo.[fr/net/…]`** : MX, SPF, DMARC, DKIM publié,
  relais sortant. C'est le seul trou à ouvrir dans l'egress (D12), et il
  fera l'objet d'une RFC à part.
- **Netbird :** IMAP et submission depuis le poste sans passer par Traefik.
- **`StalwartSieveScript`** (filtres par compte) et boîtes partagées avec
  ACL.
- **La brique de liaison entre opérateurs**
  ([RFC 0002](https://github.com/batleforc/weebo-si/blob/develop/0.docs/content/docs/rfc/0002-liaison.md)) :
  `AuthentikUser` → `StalwartAccount`, départ → `state: Locked`, groupes
  Authentik → `StalwartGroup`. Elle consomme le contrat de *Ce que la
  brique de liaison consomme*.
- **SCIM**, si une licence Enterprise arrive. Il ferait, pour Stalwart seul,
  le travail de la brique de liaison.
- **Une `StalwartLocalInstance` utilisée**, si une équipe veut son propre
  Stalwart : le kind existe dès la v0.1.0 (D2), il ne manque que le
  déploiement.
- **Ce que le spike pousserait dans le kit** : la lecture par `remoteId`
  dans le template (D9), l'unicité à l'admission si un second opérateur la
  veut (D16), un client JMAP générique si un second opérateur JMAP arrive
  (règle des deux opérateurs).

## Plan d'implémentation

Chaque case est une PR qu'on peut merger seule.

**Spike (avant `Accepted`)**

- [ ] Stalwart v0.16.27 en podman, avec un Authentik de test ou un IdP
      mock : réponse à Q1, et confirmation sur l'image de ce qui est lu
      dans le source (Q2–Q5, Q8–Q10), consignées dans le changelog de cette
      RFC

**Phase 0 — weebo-si, sans opérateur**

- [ ] `weebo-si:2.terra` : `mail.tf` (`mv/mail/{pg,admin}`, rôles
      `mail-read` et `stalwart-operator` avec la policy de D15), `s3.tf`
      (`mv/mail/s3`)
- [ ] `weebo-si:2.argo/helm/database/sub` : `- name: mail` dans
      `s3.mainInstance.creds` (user, bucket et policy RustFS)
- [ ] `weebo-si:2.argo/helm/auth/sub` : application `stalwart` (oauth2,
      `mv/mail/auth`)
- [ ] `weebo-si:2.argo/app` : `templates/mail.yaml` (Application `mail`),
      entrée `mail` des values ; AppProject `dev-base` : repo
      `oci://ghcr.io/batleforc/charts/weebo-stalwart`, destinations `mail`
      et `weebo-stalwart`, CRD et webhook de weebo-stalwart dans
      `clusterResourceWhitelist`, comme weebo-forgejo
- [ ] `weebo-si:2.argo/helm/mail` : SecretStore, ExternalSecrets
      `mv/mail/{pg,s3,admin,auth}`, CNPG, Certificate, StatefulSet,
      Services, IngressRoute, CiliumNetworkPolicy (egress vers Traefik)
- [ ] Job PostSync et CronJob `stalwart-bootstrap` (D17) : plan `apply`
      (BlobStore, Directory, domaine, compte de l'opérateur), ApiKey
      revalidée chaque jour, Secret `stalwart-operator-token` dans `mail`
- [ ] Forgejo branché en SMTP sur un compte du plan, reset de mot de passe
      testé de bout en bout

**Phase 1 — weebo-stalwart v0.1.0**

- [ ] Dépôt rendu par `task template:render` /
      `cargo generate --tag v0.18.x templates/operator` (`product = Stalwart`),
      sans retrait : `StalwartInstance`, `StalwartLocalInstance`, `Status`
      et `ReasonCode` du template ; `cog.toml` du kit, `checks.yml` /
      `publish.yml` par tag, chart sur `weebo-operator-lib`
- [ ] Repris de weebo-forgejo : `secretTargets` (`emit`,
      `SecretStoreFactory`), `ClusterReader::claimants`, crate `importer`,
      `lastRotation` et `address` dans le `Status`
- [ ] `task test:all`, `task audit` et le budget `weebo-kit-perf` verts
      dès la première PR
- [ ] Client JMAP (`POST /jmap`) et test de contrat contre l'image v0.16.27,
      dont « reste silencieux » contre le vrai serveur et un test par
      normalisation de *Champs comparés*
- [ ] `StalwartInstance` (`ping`, hash de schéma), `StalwartNamespacePolicy`,
      webhook
- [ ] `StalwartDomain` (cluster-scoped, dimension `domains`, `Orphan` seul)
- [ ] `weebo-stalwart-importer` (`import-pending`, garde
      `RemoteObjectClaimed`) pour reprendre le domaine et les comptes de la
      phase 0, rotation forcée des comptes d'envoi (D21)
- [ ] `StalwartServiceAccount` : `emit` (`write_all`, `any_missing`,
      `rotation_requested`), dimension `names`, cible Vault `smtp`
- [ ] weebo-si : opérateur en wave 3, comptes de forgejo, Authentik et
      Argo CD en CR (Argo CD par `weebo-si:1.pulu-conf/main.go`), retrait
      du plan et des ressources Terraform pour ces objets

**Phase 2 — weebo-stalwart v0.2.0**

- [ ] `StalwartAccount` (`state`, verrouillage, `Locked` = `Ready: True`)
- [ ] `StalwartGroup`, `StalwartMailingList`
- [ ] weebo-si : un `StalwartAccount` déclaré à la main dans
      `weebo-si:2.argo/helm/mail`, verrouillage vérifié (App Password créé
      dans la WebUI, refusé verrouillé, accepté déverrouillé) ; le reste des
      users attend la brique de liaison

**Clôture**

- [ ] Doc : Big Picture Mail, page Macro, doc de l'opérateur
      (`@weebo/operator-docs`), left-todo mis à jour
- [ ] RFC passée en `Implemented`

## Références

- [Stalwart v0.16 : l'API de gestion passe en JMAP](https://stalw.art/blog/stalwart-0-16) ;
  source `stalwartlabs/stalwart` `main` `aa47d8e` (v0.16.27) :
  `crates/http/src/request.rs` et `crates/http/src/api/mod.rs` (routes
  `/jmap`, `/api`), `crates/common/src/cache/directory.rs` (JIT),
  `crates/directory/src/backend/oidc/lookup.rs`,
  `crates/jmap/src/registry/set.rs` et `mapping/account.rs`
  (`impersonate`, création des App Passwords et ApiKeys),
  `crates/scim/src/users/mod.rs` (`set_active`) ;
  `stalwartlabs/cli` (`--api-key`, `/jmap/session`)
- [Directory OIDC et ses limites](https://stalw.art/docs/auth/backend/oidc/)
- [Index du schéma](https://www.stalw.art/docs/ref/) :
  [Account](https://www.stalw.art/docs/ref/object/account/),
  [Domain](https://www.stalw.art/docs/ref/object/domain/),
  [MailingList](https://www.stalw.art/docs/ref/object/mailing-list/),
  [ApiKey](https://www.stalw.art/docs/ref/object/api-key/),
  [AppPassword](https://www.stalw.art/docs/ref/object/app-password/),
  [BlobStore](https://www.stalw.art/docs/ref/object/blob-store/),
  [DataStore](https://www.stalw.art/docs/ref/object/data-store/),
  [Certificate](https://www.stalw.art/docs/ref/object/certificate/),
  [MtaStageAuth](https://www.stalw.art/docs/ref/object/mta-stage-auth/),
  [Role](https://www.stalw.art/docs/ref/object/role/),
  [Permissions](https://www.stalw.art/docs/ref/permissions/)
- [CLI](https://www.stalw.art/docs/management/cli/) et
  [`apply`](https://www.stalw.art/docs/management/cli/apply/),
  [déploiements déclaratifs](https://www.stalw.art/docs/configuration/declarative-deployments/)
- [Kubernetes (chart d'exemple, StatefulSet)](https://stalw.art/docs/cluster/orchestration/kubernetes)
- Providers Terraform communautaires : `flungo/terraform-provider-stalwart`,
  `tahacodes/terraform-provider-stalwart`
- weebo-operator-kit : `docs/plan.md` (« Ce qui va dans le kit »,
  garde-fous, NamespacePolicy, v0.15.2, v0.17.0), `docs/rfc-vault-path-grants.md`,
  `templates/operator` (`reconcile_object.rs`, `domain/src/error.rs`),
  `crates/weebo-kit-api` (`InstanceRef`, `ObjectRef`, `SecretTarget`,
  annotations), `crates/weebo-kit-domain/src/allow_list.rs`,
  `crates/weebo-kit-domain/src/drift.rs`,
  `crates/weebo-kit-application/src/emit.rs`,
  `crates/weebo-kit-adapters/src/{secret_routing,secret_vault}.rs`,
  `crates/weebo-kit-runtime/src/admission.rs`,
  `charts/weebo-operator-lib/templates/_monitoring.tpl`
- weebo-forgejo : `crates/api/src/status.rs` (`lastRotation`),
  `crates/adapters-outbound/src/cluster_reader.rs` (`claimants`),
  `crates/importer`
- weebo-si : `2.argo/helm/forgejo` (opérateur, Job token, NamespacePolicy,
  `secretTargets`, waves), `2.argo/helm/batlehub` (mode external),
  `2.argo/helm/auth/sub` (applications, users), `2.argo/helm/database/sub`
  (`s3.mainInstance.creds`), `2.argo/app` (Applications, AppProjects),
  `2.terra/vault/terra-map/{forgejo,s3}.tf`, `1.pulu-conf/main.go`
  (Argo CD), `2.argo/helm/monitoring/dashboards/README.md`
- RFC de weebo-si :
  [0002 — brique de liaison](https://github.com/batleforc/weebo-si/blob/develop/0.docs/content/docs/rfc/0002-liaison.md),
  [0003 — n'écrire que ce qui diffère](https://github.com/batleforc/weebo-si/blob/develop/0.docs/content/docs/rfc/0003-authentik-ecritures.md)
- weebo-si-hardening : gabarit RFC (`docs/rfc/0000-template.md`)

## Changelog

| Date | Changement |
| --- | --- |
| 2026-10-18 | Brouillon initial |
| 2026-10-18 | Alignement strict sur weebo-operator-kit v0.17.0 : `LocalInstance` gardée, `StalwartDomain` cluster-scoped (`ObjectRef` sans namespace), dimensions d'allow-list `domains`/`names` au lieu de `reservedNames`, plus d'adoption par nom (importer + `import-pending`), `deletionPolicy` limitée à `Delete`/`Orphan`, `rotate-secret` en nonce, `Status` et codes du template, plus de `basicAuthSecretRef` ni de `versionRange`, `pathPrefix: ""` sans `pathGrants` |
| 2026-10-18 | Lien Authentik → Stalwart sorti de l'opérateur : `StalwartAccountSource`, reflector `AuthentikUser`, RBAC sur `authentik.weebo.io` et codes `SourceOutsideScope`/`ChildrenNotReady` retirés. Une autre brique, qui lie les opérateurs entre eux, consomme les `StalwartAccount` (D7, alternative 5) |
| 2026-10-18 | Revue du 2026-10-18 appliquée ; RFC déplacée dans weebo-operator-kit. Kit v0.18.x et modèle de la RFC 0003 (*Champs comparés*, « reste silencieux » contre l'image) ; API par `POST /jmap` ; version par hash de schéma (D13) ; `StalwartAppPassword` repoussé, App Passwords en libre-service (D14) ; policy Vault `mv/{data,metadata}/+/smtp` et invariant sur les chemins sans owner (D15) ; unicité au reconcile (D16) ; jeton par Secret Kubernetes revalidé chaque jour (D17) ; `StalwartDomain` en `Orphan` seul (D18) ; garde d'identité sur `remoteId` (D19) ; `Locked` = `Ready: True` et App Passwords gardés (D20) ; rotation à l'import (D21) ; port 25, sonde et alertes retirés de la v1 (D22) ; types Stalwart corrigés (`roles`, `encryptionAtRest`, alias, quotas, `subAddressing`, DKIM, BlobStore, `recipients`) ; allow-list par namespace et `Deny` sur tous les kinds ; Q2–Q6, Q8, Q9 tranchées ou réduites, Q10–Q11 ajoutées ; plan Argo, Vault, bucket et Argo CD corrigés |
