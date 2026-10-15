# RFC — Chemins Vault hors `<pathPrefix>/<namespace>/` sur autorisation de l'instance

- **Dépôt :** weebo-operator-kit (puis weebo-authentik, weebo-forgejo)
- **Base :** kit v0.14.3, weebo-authentik v0.17.2, weebo-forgejo v0.3.2
- **Statut :** acceptée, 2026-10-14 (décisions en fin de document) ;
  partie kit implémentée sur `docs/rfc-vault-path-grants`
- **Release visée :** kit v0.15.0 (mineure, cassante pour la compilation des opérateurs)

## Résumé

Depuis la migration de weebo-authentik sur le kit (v0.16.0), un CR namespacé
ne peut écrire une cible Vault que sous `<pathPrefix>/<son namespace>/`,
`path` explicite compris. La règle est juste pour des CR posés par des
équipes, mais elle rend impossible le cas d'usage pour lequel les
`secretTargets` Vault ont été ajoutés : un admin qui déclare toutes les
applications dans un namespace (`auth`) et publie chaque credential à
l'endroit où son consommateur le lit déjà (`mv/batlehub/auth`,
`mv/argocd/auth`…).

Cette RFC ajoute à la connexion Vault de l'instance **cluster** une liste
d'autorisations, `pathGrants`, qui permet à des namespaces nommés d'écrire
des chemins nommés hors de leur sous-arbre. Sans `pathGrants`, rien ne change
pour les CR namespacés. Elle corrige au passage `pathPrefix: ""`, qui
aujourd'hui refuse tout chemin explicite, et soumet les owners
cluster-scoped, libres jusqu'ici, à la même règle (aucun kind cluster
n'émet de secret aujourd'hui, donc rien ne casse).

## Contexte

### La règle actuelle

`weebo-kit-adapters/src/secret_routing.rs`, `RoutingSecretStore::vault_path` :

```rust
if let Some(path) = &target.path {
    if let Some(ns) = owner.namespace {
        let prefix = vault.namespace_prefix(ns);          // "<pathPrefix>/<ns>/"
        let normalized = path.trim_start_matches('/');
        if !normalized.starts_with(&prefix) || normalized.split('/').any(|s| s == "..") {
            return Err(/* "outside {prefix}, the only subtree a CR in namespace {ns} may write to" */);
        }
    }
    return Ok(path.clone());
}
```

- Un **owner cluster-scoped** (`owner.namespace == None`) n'est pas
  contraint : son `path` explicite est accepté tel quel.
- Un **owner namespacé** est enfermé dans `<pathPrefix>/<ns>/`, et aucune
  configuration ne permet d'en sortir.
- `namespace_prefix` et `default_path` font un `format!("{}/{ns}/…",
  path_prefix)`. Avec `pathPrefix: ""`, le préfixe vaut `/<ns>/`, alors que
  le chemin est normalisé sans `/` initial : **aucun** chemin explicite ne
  passe, et le chemin par défaut commence par `/`.

La raison d'être est écrite dans `docs/plan.md` (« sinon un locataire lit les
chemins des autres avec l'identité de l'opérateur ») : l'identité Vault de
l'opérateur est partagée par tous les namespaces, donc sans cette borne,
quiconque peut créer un `AuthentikApplication` dans un namespace autorisé
écrit où l'opérateur peut écrire.

### Ce qui casse chez un utilisateur réel (weebo-si)

Les `AuthentikApplication` vivent toutes dans `auth` et publient vers les
chemins que leurs consommateurs lisent depuis l'époque Terraform :

| Application | `path` | Lu par |
| --- | --- | --- |
| argo | `argocd/auth` | ArgoCD (namespace `argocd`) |
| che | `eclipse-che/auth`, `dex/auth` | Che, Dex |
| angos | `angos/auth` | angos |
| batlehub | `batlehub/auth` | batlehub |
| forgejo | `forgejo/auth` | Forgejo |
| endpoint-gateway | `weebo-si-hardening/endpoint-gateway` | endpoint-gateway |
| s3 | `s3/auth` | RustFS |

Chaque consommateur lit avec un `SecretStore` ESO dont la policy Vault est
templatée sur son propre namespace (`mv/data/<ns>/*`) : déplacer les
chemins sous `weebo-authentik/auth/` oblige soit à élargir la lecture de
chaque consommateur, soit à un `ClusterSecretStore`, ce qui revient à
retirer l'isolation que la règle du kit voulait protéger.

Sur 0.17.2, toutes ces applications passent `Ready: False`
(`SecretStoreError`). Les valeurs déjà présentes restent, mais plus rien
n'est réécrit, et une application nouvelle (forgejo) n'est jamais publiée.
Le même plafond s'applique à weebo-forgejo dès qu'un CR y pose une cible
Vault.

## Proposition

### 1. `pathGrants` sur la connexion Vault de l'instance cluster

```yaml
apiVersion: authentik.weebo.io/v1alpha1
kind: AuthentikInstance            # cluster-scoped : RBAC d'admin seulement
metadata:
  name: main
spec:
  secretStore:
    backend: kubernetes
    vault:
      address: https://openbao.vault:8200
      mount: mv
      pathPrefix: weebo-authentik
      kubernetesAuthRole: auth
      # Nouveau. Absent ou vide : comportement actuel.
      pathGrants:
        - namespaces: [auth]
          paths:
            - argocd/auth
            - eclipse-che/auth
            - dex/auth
            - angos/auth
            - batlehub/auth
            - forgejo/auth
            - weebo-si-hardening/endpoint-gateway
            - s3/auth
```

**Règle d'évaluation** pour le `path` explicite d'un owner namespacé `ns` :

1. On le normalise : `/` initiaux retirés. Il est refusé s'il est vide, s'il
   contient un segment vide (`a//b`, `a/`) ou un segment `.` ou `..`. Ce
   contrôle existe déjà en partie et devient commun aux deux branches. Le
   chemin écrit est le chemin normalisé.
2. Accepté s'il est sous `<pathPrefix>/<ns>/` (règle actuelle, inchangée).
3. Sinon, accepté si **un** grant a un motif de `namespaces` qui couvre
   `ns` **et** une entrée de `paths` qui couvre le chemin.
4. Sinon, refusé. Le message dit ce qui manque :
   `target "t": Vault path "x/y" is outside "weebo-authentik/team-a/" and no pathGrants entry of the instance allows it for namespace "team-a"`.
   Le store ne connaît pas le nom de l'instance (la connexion Vault est
   partagée par clé) : le message dit « the instance ».

**Motifs :**

- `namespaces` : même syntaxe que les `*NamespacePolicy` (nom exact, `*`, ou
  préfixe `team-*`), via le `namespace_matches` de
  `weebo-kit-domain::allow_list`, rendu public pour l'occasion. Une seule
  grammaire de motif de namespace dans tout le kit.
- `paths` : une suite de segments séparés par `/`, comparée **segment par
  segment** (jamais comme un préfixe de chaîne). Même vocabulaire que les
  policies Vault :
  - un segment littéral ne couvre que lui-même, et ne contient ni `*` ni
    `+` (`a*` est refusé, pas lu comme un glob) ;
  - `+` couvre **exactement un** segment, quel qu'il soit, à n'importe
    quelle position : `+/auth` couvre `batlehub/auth` et `angos/auth`,
    mais ni `auth`, ni `a/b/auth`, ni `batlehub/auth/x` ;
  - `*` n'est permis qu'en **dernier** segment, et couvre un ou plusieurs
    segments : `batlehub/*` couvre `batlehub/auth` et `batlehub/a/b`, mais
    pas `batlehub` seul ni `batlehubx/…`.

  Un motif doit contenir **au moins un segment littéral**. `*`, `+`, `+/*`
  et `+/+` sont refusés : un grant « tout le mount » ne s'écrit pas par
  accident. Les segments vides, `.`, `..` et un `/` initial sont refusés,
  comme dans les chemins.

**Ce que `pathGrants` ne change pas :**

- Les chemins **par défaut** (`<pathPrefix>/<ns>/<name>`) et la règle de
  namespace des cibles Kubernetes (`emit::check_target`).
- La **propriété** : un chemin dont la métadonnée `owner` désigne un autre
  CR reste refusé (`SecretStoreError`), grant ou pas.
- Les owners **cluster-scoped** ont leur propre règle : voir la partie 3.
- La `*LocalInstance`, qui n'a pas de Vault (kit étape 3). Le type n'est
  **pas** ajouté à son spec, et le confinement prévu dans `plan.md` reste
  valable.

### 2. `pathPrefix: ""`

`namespace_prefix` et `default_path` joignent leurs segments en sautant un
préfixe vide : `""` donne `<ns>/` et `<ns>/<name>`. Les opérateurs gardent
leur défaut (`weebo-authentik`, `weebo-forgejo`). Un admin dont la
convention Vault est déjà `<ns>/…` peut alors s'en passer, et les chemins
par défaut tombent là où une policy templatée sur le namespace les lit.
C'est une correction de bug : la valeur était acceptée par le schéma mais
inutilisable.

**Attention (revue du 2026-10-15)** : avec `pathPrefix: ""`, le sous-arbre
d'un namespace est un dossier de premier niveau du mount. Un namespace qui
porte le nom d'un dossier existant (`shared`, ou le premier segment d'un
chemin ouvert par un grant) possède ce dossier : ses objets y écrivent
sans grant, et peuvent y adopter les valeurs sans owner. Un préfixe vide
ne convient qu'à un mount réservé aux dossiers par namespace, où personne
ne peut créer un namespace du nom d'un autre dossier de premier niveau.
L'avertissement est repris dans la doc de `VaultConfig::path_prefix` et de
`vault_paths::subtree` ; chaque opérateur le reprend dans la description
de son champ `pathPrefix`.

### 3. Owners cluster-scoped

**Constat (kit v0.14.3, authentik 0.17.2, forgejo 0.3.2) :** aucun kind
cluster-scoped ne porte de `secretTargets`. Côté authentik, `AuthentikUser`,
`AuthentikGroup`, `AuthentikFlow` et les autres n'émettent rien. Côté
forgejo, tous les kinds émetteurs (`ForgejoUser`, `ForgejoAccessToken`,
`ForgejoOAuth2Application`, `ForgejoUserKey`, `ForgejoDeployKey`) sont
namespacés. La branche « owner cluster, chemin libre » de `vault_path`
n'est donc empruntée par personne.

**On les borne maintenant.** Resserrer aujourd'hui ne casse rien.
Resserrer plus tard, une fois qu'un kind cluster émetteur existera, serait
une rupture, alors qu'un assouplissement ultérieur ne l'est jamais.

- Un owner cluster écrit sous `<pathPrefix>/<target.namespace>/` (la cible
  doit nommer un namespace, `MissingNamespace` sinon), ou sur un chemin
  couvert par un grant qui l'accepte explicitement :

  ```yaml
  pathGrants:
    - clusterScoped: true      # au lieu de `namespaces`, exclusif
      paths: [platform/+/credentials]
  ```

  Même idée que le `sharedWith` des `LocalInstance`, qui accepte les objets
  cluster à part.
- Un owner cluster avec un `path` explicite **sans** `namespace` n'est
  possible que via un grant `clusterScoped`.
- La normalisation (`..`, `//`, `/` initial) s'applique à tous les owners.

Un grant porte donc **soit** `namespaces`, **soit** `clusterScoped: true`,
jamais les deux (règle CEL).

## Conception dans le kit

Les couches suivent les règles de `CLAUDE.md` : ce qui est pur va dans le
domaine, et rien ne nomme un produit.

| Crate | Changement |
| --- | --- |
| `weebo-kit-api` | `VaultPathGrant { namespaces: Vec<String>, cluster_scoped: bool, paths: Vec<String> }` (serde camelCase, schemars) et `PATH_PATTERN`. Les doc comments deviennent la description CRD et parlent de « l'instance ». Les opérateurs l'embarquent dans leur `VaultSecretStoreSpec`, **avec une borne** : `#[schemars(length(max = 32))]` sur le champ `pathGrants` (voir Validation). |
| `weebo-kit-domain` | `vault_paths` : `normalize`, `subtree(prefix, ns)` et `default_path(prefix, ns, name)` (préfixe vide sauté), `granted(grants, writer, path)`, `validate(grants) -> Result<(), GrantError>`, et `check(prefix, grants, writer, namespace, path) -> Result<&str, PathDenied>`, l'unique point d'entrée des stores et des webhooks, qui rend le chemin normalisé. Les grants y sont une vue empruntée `PathGrant<'a>` (le domaine ne dépend pas de l'API), l'écrivain un `Writer::Namespaced(ns)` ou `Writer::Cluster`. Un motif invalide ne couvre rien. `allow_list::namespace_matches` devient `pub`. |
| `weebo-kit-adapters` | `VaultConfig.path_grants: Vec<VaultPathGrant>`, recopié dans `VaultSecretStore`, validé dans `VaultSecretStore::new` **avant** le login. `VaultSecretStore::check_path` délègue à `vault_paths::check` ; `RoutingSecretStore::vault_path` l'appelle pour tous les owners (la branche « cluster, chemin libre » disparaît). `namespace_prefix` et `default_path` délèguent au domaine. `VaultConfig` fait partie de la clé du cache `VaultLogins` : modifier les grants refait le login, sans effet visible. |
| `weebo-kit-application` | Rien : `check_target` reste la règle de namespace, indépendante de Vault. |
| `weebo-kit-runtime` | `admission::check_vault_targets(targets, namespace, vault: Option<VaultPaths>)` : `VaultPaths { path_prefix, path_grants }` est la vue Vault de l'instance résolue (le kit ne connaît pas les kinds), `None` quand elle n'a pas de Vault. Renvoie le message du store (`secret target "t": …`), que l'opérateur met dans son `Denial` avec sa raison. |
| `templates/operator` | Le gabarit n'a pas de secret store : rien à faire. Une note dans `plan.md` suffit. |
| `packages/operator-docs` | Rien : le générateur de pages CRD reprend les descriptions du schéma. Le guide (« Writing outside the namespace subtree ») va dans les pages d'opérateur (`connect-instance.mdx`), qui ne vivent pas dans le kit. |

**Validation des grants, à trois niveaux :**

1. **Schéma CRD.** Le `JsonSchema` de `VaultPathGrant` porte, par entrée
   de `paths`, le `pattern` `PATH_PATTERN` (segments non vides, `*` et `+`
   seulement en segment entier, `*` seulement en dernier, pas de `/`
   initial) et deux règles CEL, faute de lookahead dans les regex de
   l'apiserver : `!self.matches('(^|/)\\.\\.?(/|$)')` (ni `.` ni `..`) et
   `self.matches('(^|/)[^/*+]+(/|$)')` (au moins un segment littéral).
   `paths` a `minItems: 1`, et une règle CEL sur le grant impose
   exactement l'un de `namespaces` et `clusterScoped`. Un motif mal formé
   est refusé par l'apiserver, sans webhook.

   **Bornes.** Les règles CEL sont dans des listes : l'apiserver estime
   leur coût sur la taille maximale des listes et des chaînes, et refuse
   la CRD si l'estimation dépasse son budget. `VaultPathGrant` borne ses
   listes (`paths` ≤ 64 entrées de 512 caractères, `namespaces` ≤ 64), et
   l'opérateur doit borner `pathGrants` (`#[schemars(length(max = 32))]`).
   Vérifié sur un apiserver envtest (kube 1.36) : avec la borne, la CRD est
   acceptée et la matrice de motifs se comporte comme `validate` ; sans
   elle, la CRD est refusée à l'installation (l'envtest de l'opérateur
   l'attrape).
2. **`vault_paths::validate`** à la construction du `VaultSecretStore`,
   comme filet si le schéma a été contourné (CRD ancienne). Un grant
   invalide fait échouer la connexion avant tout login, avec un message
   explicite (`pathGrants[i]: …`), et les CR passent `SecretStoreError`.
3. **Reconcile de l'instance.** L'opérateur appelle le même `validate`
   pour mettre l'instance en `Ready: False` avec une raison à lui (le kit
   ne définit pas de code de raison).

**Admission des CR.** Le `/validate` vérifie les chemins Vault explicites
des `secretTargets` à la création et à la mise à jour, avec la même
fonction `vault_paths::check` que les stores :

- l'instance est résolue comme le fait le reconcile : `status.instance`
  enregistrée, sinon `instanceRef`, sinon l'instance par défaut. Le
  webhook lit les réflecteurs d'instances déjà présents dans le binaire
  (kit étape 2), donc **aucun appel API** de plus par requête ;
  - instance introuvable ou ambiguë : pas de refus, le reconcile la
    signalera (`InstanceNotFound`, `AmbiguousDefaultInstance`), comme
    aujourd'hui ;
  - instance trouvée sans `secretStore.vault` alors qu'une cible est
    Vault : refus ;
- le message de refus est celui du store, préfixé du kind et du nom ;
- chaque opérateur appelle `admission::check_vault_targets` du runtime
  (qui passe par `vault_paths::check`) depuis son `Admission::admit` ;
- la vérification au reconcile reste : si on retire un grant de
  l'instance, les CR déjà admis le découvrent au prochain reconcile
  (`SecretStoreError`). Réadmettre tout le monde à chaque changement
  d'instance n'est pas prévu.
- la suppression aussi : un CR dont le chemin n'est plus couvert reste en
  `Terminating` (`SecretStoreError`) tant que le grant n'est pas remis.
  Comportement accepté : retirer un grant laisse la valeur en place
  plutôt que de l'orpheliner en silence.

Les CR liés à une `LocalInstance` n'ont pas de cible Vault (refus existant)
et ne sont pas concernés.

## Sécurité

- **Qui peut accorder :** quiconque peut écrire l'instance cluster, donc les
  admins du cluster (ou le projet ArgoCD qui la porte). Ce sont les mêmes
  sujets qui créent les CR cluster-scoped, désormais bornés eux aussi
  (partie 3).
- **Ce qu'il ouvre :** à **tout** sujet autorisé à créer le kind dans un
  namespace couvert. Le grant se combine donc avec RBAC et la
  NamespacePolicy, il ne les remplace pas. La doc doit le dire : un grant
  sur `team-*` ouvre ces chemins à toutes les équipes.
- **Adoption des valeurs sans propriétaire.** Une valeur Vault sans
  métadonnée `owner` compte comme celle de son premier écrivain (adoption
  des écritures d'avant la propriété). Un grant sur un chemin qui contient
  déjà un secret non géré (par exemple `main-config`) permettrait à un CR
  du namespace couvert de l'**écraser**. C'est la raison des motifs
  étroits. Un `+` en tête (`+/auth`) couvre **chaque** chemin de cette
  forme, y compris ceux écrits par Terraform et jamais adoptés : il est
  pratique quand un namespace publie pour tous ses consommateurs, mais la
  doc recommande des chemins exacts dès qu'un sous-arbre contient des
  secrets non gérés. Un motif sans segment littéral est refusé (voir
  Motifs).
- **La borne réelle reste Vault.** Les grants empêchent l'opérateur
  d'écrire ce qu'il pourrait écrire. La policy du rôle Kubernetes de
  l'opérateur reste la limite dure, et la doc recommande de la restreindre
  aux mêmes chemins. Dans weebo-si, le rôle `auth` porte `mv_policy` (KV
  entier plus `sys/` et `auth/`) : le restreindre est un chantier à part.

## Alternatives écartées

1. **Interrupteur global** (`pathScope: Cluster` ou `restrictPaths:
   false`). Le plus simple, mais tout ou rien : chaque namespace autorisé
   par la NamespacePolicy écrirait partout, adoption de `main-config`
   comprise.
2. **Une dimension dans la `*NamespacePolicy`** (`allowedVaultPaths`). Le
   « qui peut quoi » serait au même endroit que le reste, mais :
   - la policy est optionnelle et vaut « tout permis » quand elle est
     absente ;
   - elle est sautée pour les CR liés à une `LocalInstance` ;
   - le moteur d'allow-list raisonne sur des kinds et des références, pas
     sur des destinations de backend ;
   - la connexion Vault vit sur l'instance.

   Les bornes d'un backend vont avec sa configuration.
3. **Des CR cluster-scoped** pour les applications d'admin. Un nouveau kind
   par opérateur, dupliqué : beaucoup de surface pour une règle de chemin.
4. **Ne rien changer au kit et déplacer les chemins.** Voir le contexte :
   soit on élargit la lecture de chaque consommateur, soit on passe par un
   `ClusterSecretStore`. L'isolation est perdue côté lecture pour la
   préserver côté écriture.
5. **Déplacer les CR dans le namespace de leur consommateur** (avec
   `pathPrefix: ""`, que la partie 2 rend possible). Viable pour la
   plupart, mais pas pour une application qui alimente deux namespaces
   (che : `eclipse-che` et `dex`), et cela disperse l'allow-list et les
   AccessPolicy. Ça reste une option pour les utilisateurs qui le
   préfèrent.

## Compatibilité et versions

- **Kit v0.15.0 (mineure).** `VaultConfig` gagne un champ, ce qui casse la
  construction littérale chez les opérateurs (règle de `CLAUDE.md` : un
  changement qui casse la compilation d'un opérateur est une mineure en
  0.x). Le comportement est inchangé sans grants, à deux détails près :
  le chemin explicite écrit est normalisé (`/` initiaux retirés), et un
  segment `.` est refusé comme `..`.
- **Opérateurs :** un champ CRD optionnel (`pathGrants`, `default` vide,
  `skip_serializing_if = "Vec::is_empty"`). Ajout seulement, aucune
  migration, CRD à régénérer.
  - weebo-authentik 0.18.0, version mineure parce que le kit en est une ;
  - weebo-forgejo 0.4.0, pour les mêmes raisons et pour rester alignés.
- **`pathPrefix: ""`** : seuls les chemins écrits avec un préfixe vide
  changent, et ils ne pouvaient pas fonctionner jusque-là.
- **Owners cluster-scoped** : la règle se resserre, mais aucun kind des deux
  opérateurs n'est concerné (voir le constat de la partie 3). Un futur kind
  cluster émetteur naîtra directement sous la nouvelle règle.

## Tests

- **Domaine (unitaires) :**
  - matrice de `normalize`, `under_prefix` et `granted` : exact,
    sous-arbre, préfixe de chaîne non aligné sur les segments, `..`, `//`,
    `/` initial, préfixe vide, namespace couvert par `*` et par `team-*` ;
  - `+` : `+/auth` couvre `a/auth`, mais ni `auth` ni `a/b/auth` ;
    `a/+/c` ; `+` combiné avec un `*` final (`+/auth/*`) ;
  - `validate` refuse `*`, `+`, `+/*`, `+/+`, `a/*/b` (`*` pas en dernier),
    une liste vide, un chemin avec `..`, et un grant qui porte à la fois
    `namespaces` et `clusterScoped` (ou aucun des deux) ;
  - owner cluster : sous `<pathPrefix>/<target.namespace>/` accepté ; hors
    de ce sous-arbre, refusé sans grant `clusterScoped` et accepté avec ;
    un grant `namespaces: ["*"]` ne couvre **pas** un owner cluster.
- **Schéma :** `weebo-kit-adapters/tests/grant_schema.rs` compile le
  `pattern` et les regex des règles CEL générés, et vérifie sur une
  matrice qu'un motif est accepté par le schéma si et seulement si
  `validate` l'accepte. La règle d'exclusion et le budget CEL ont été
  vérifiés une fois sur envtest (voir Validation) ; l'envtest des
  opérateurs, qui installe leurs CRD, le refait à chaque build.
- **`routing_contract.rs` (wiremock) :**
  - un CR namespacé avec un grant écrit hors de son sous-arbre ;
  - sans grant, il est refusé avec le message attendu ;
  - avec grant mais sur une valeur appartenant à un autre owner, il est
    refusé ;
  - avec `pathPrefix: ""`, le chemin par défaut vaut `<ns>/<name>` ;
  - un owner cluster avec un `path` libre et sans grant est refusé, et
    accepté avec un grant `clusterScoped` (il n'existait pas de test
    « cluster owner writes anywhere »).
- **`vault_contract.rs` :** un grant invalide fait échouer la connexion
  sans aucun login ; le reste est inchangé.
- **Runtime :** `check_vault_targets` refuse comme le store, et refuse une
  cible Vault quand l'instance n'a pas de Vault.
- **Opérateurs (envtest) :**
  - une `AuthentikApplication` dans `auth` avec `path: batlehub/auth` passe
    `Ready` avec un grant et `SecretStoreError` sans (même chose pour un
    `ForgejoOAuth2Application`) ;
  - admission : le même CR sans grant est **refusé à l'apply** ;
  - un CR dont l'instance n'existe pas encore est admis ;
  - un grant retiré après admission donne `SecretStoreError` au reconcile.

## Déploiement

1. PR kit (`feat!: vault path grants on the cluster instance`), puis tag
   v0.15.0.
2. weebo-authentik : passage au kit v0.15.0, champ CRD (borné à 32),
   `path_grants` recopié dans le `VaultConfig` de l'instance cluster (vide
   pour une `LocalInstance`), `check_vault_targets` dans le `/validate`,
   validation dans le reconcile de l'instance, docs
   (`connect-instance.mdx`, section « Where credentials go »), release
   0.18.0.
3. weebo-forgejo : même chose, release 0.4.0.
4. weebo-si : ajouter `pathGrants` dans `2.argo/helm/auth/sub`
   (`foundation.vault`), puis dérouler le plan de montée 0.15.0 → 0.18.0
   (`instanceRef` objet, `secretTargets[].name`, `migrate-instance-ref`).

## Décisions

1. **Nom du champ** : `pathGrants`.
2. **Syntaxe des chemins** : segments littéraux, `+` pour exactement un
   segment à n'importe quelle position, `*` seulement en dernier segment.
   Au moins un segment littéral par motif.
3. **Owners cluster-scoped** : bornés eux aussi, sous
   `<pathPrefix>/<target.namespace>/` ou via un grant `clusterScoped: true`
   (partie 3).
4. **Admission** : les chemins Vault explicites sont vérifiés dès
   `/validate`, avec la même fonction que les stores et les réflecteurs
   d'instances (pas d'appel API).
5. **Grant retiré** : le reconcile **et** la suppression des CR concernés
   échouent (`SecretStoreError`) jusqu'à ce que le grant soit remis.
