# RFC 0002 — La brique de liaison : en-têtes `link.weebo.io`, `weebo-link` et hooks d'admission

- **Dépôts :** weebo-si (déploiement, règles), `weebo-link` (nouveau
  contrôleur). Aucun opérateur produit n'est modifié. La RFC vit ici, dans
  weebo-operator-kit, depuis le 2026-10-18.
- **Base :** weebo-operator-kit v0.18.0 (runtime, `weebo-kit-domain::drift`,
  `<prefix>_remote_writes_total`, `DriftCorrected` ; la RFC 0001 vise
  v0.18.x), Kubernetes v1.36.2 et `ValidatingAdmissionPolicy` (GA),
  weebo-authentik 0.18.3 (déployé, sur le kit v0.18.0), weebo-stalwart
  (RFC 0001)
- **Premier lien :** `AuthentikUser` (cluster-scoped) → `StalwartAccount`
  (namespacé, `mail`) (RFC 0001, D7)
- **Contrainte :** aucune dépendance à Kyverno. La brique `hook` peut
  disparaître, et la liaison doit lui survivre.
- **Statut :** Draft. Auteurs : batleforc. Créée le 2026-10-18, mise à jour
  le 2026-10-18 (revue appliquée), non décidée. D11–D15, D17 et D20 sont
  proposées, à valider.
- **Brique :** `weebo-si:2.argo/helm/link`, github.com/batleforc/weebo-link

## Résumé

Chaque opérateur weebo ne connaît que son produit (RFC 0001, D7). Quand un
objet d'un produit doit en faire exister un dans un autre, par exemple un
utilisateur Authentik et sa boîte Stalwart, le lien passe par cette brique.
Elle tient en trois pièces, sans aucun CRD :

- **les en-têtes** : l'annotation `link.weebo.io/<cible>: enabled`,
  posée dans Git sur l'objet source, déclare le lien. Sur la cible, des
  labels et annotations `link.weebo.io/*` disent d'où elle vient et où en
  est le lien ;
- **`weebo-link`** : un petit contrôleur sans état. Il lit les règles de
  lien dans sa configuration (values Helm), crée les cibles et les tient à
  jour, en n'écrivant que ce qui diffère. Au départ de la source, il les
  passe dans un **état sûr** (`Locked`) au lieu de les supprimer ;
- **les hooks** : des `ValidatingAdmissionPolicy` natives de l'apiserver,
  en CEL, rendues par le même chart. Elles valident les en-têtes et
  réservent l'écriture des cibles à weebo-link. Elles n'ont ni serveur
  webhook ni moteur tiers, et tiennent même quand weebo-link est arrêté.

Cette RFC concerne quiconque relie deux briques, ou retire un utilisateur.

## Motivation

### Aujourd'hui

- La RFC 0001 sort de weebo-stalwart le suivi des `AuthentikUser`. Il faut
  quand même :
  1. qu'un utilisateur Authentik ait sa boîte avant son premier login ;
  2. qu'un utilisateur qui part voie sa boîte verrouillée.
- Aucun opérateur ne doit lire le CRD d'un autre. Mettre le lien dans
  weebo-authentik ou dans weebo-stalwart couplerait deux produits dans un
  même opérateur, et il faudrait recommencer pour chaque paire (Authentik →
  Forgejo, Authentik → Che…).
- Kyverno sait générer des objets, mais la brique `hook` n'est pas
  garantie de rester. De plus, un `generate` supprime la cible quand la
  source disparaît. Pour une boîte mail, c'est le mauvais réflexe : la boîte
  resterait active et sans propriétaire (`Orphan`), sauf à imposer une
  garde et un départ en deux temps.

### Une fois en place

- Dans `weebo-si:2.argo/helm/auth/sub/values.yaml`, `links: [stalwart]` sur un utilisateur
  donne un `StalwartAccount` `mail/<crName>` en quelques secondes.
- **Un départ, c'est `isActive: false`, en un commit** (D12). Authentik
  désactive le compte, et weebo-link passe la boîte en `state: Locked`.
  Elle reste liée (`Linked`), visible et réactivable par `isActive: true`.
- **Retirer l'utilisateur des values ne suffit pas.** L'Application
  `auth-app` tourne en `prune: false`, exprès : ses CR portent un finalizer
  qui détruit l'objet Authentik. Un `AuthentikUser` retiré de Git reste
  donc dans le cluster, actif, et sa boîte reste `Linked`. La suppression
  de l'`AuthentikUser` est un geste manuel (D12) ; alors seulement la cible
  passe `Detaching` puis `Detached`.
- Retirer seulement l'en-tête (`stalwart` sorti de `links`) se fait en un
  commit : Argo CD retire l'annotation, weebo-link verrouille la boîte et
  la garde, détachée. Si weebo-link était arrêté à ce moment, il le fait à
  son redémarrage : la cible garde la trace de sa source.
- Un nouveau lien, c'est une entrée dans les values de `weebo-si:2.argo/helm/link`,
  sans code.

### Objectifs et non-objectifs

| Objectif | Non-objectif |
| --- | --- |
| Créer un CR cible depuis un objet source annoté | Recopier des secrets (c'est `secretTargets` / ExternalSecrets) |
| Recopier quelques champs (identité, état actif) | Transformer des données de façon arbitraire, ou appeler une API produit |
| Au départ de la source, mettre la cible en état sûr, même après une panne | Supprimer des données produit |
| N'écrire la cible que si un champ du mapping diffère (RFC 0003) | Remonter un status dans la source |
| Zéro CRD, zéro dépendance tierce | Des liens définis par les équipes au runtime |
| Des liens entièrement déclarés dans Git, par les admins | Adopter une cible existante (D9) |

## Explication pour l'exploitant

### Ce qui est installé

L'Application Argo `link`, nouveau chart `weebo-si:2.argo/helm/link`. Elle passe
après les opérateurs, dont elle a besoin des CRD :

| Wave | Objet | Rôle |
| --- | --- | --- |
| 0 | ConfigMap `weebo-link-rules` | les règles, rendues depuis `links:` |
| 0 | ClusterRole / Binding | lecture des kinds source **cluster-scoped** (`authentikusers`), et rien d'autre |
| 0 | Role / RoleBinding par namespace | écriture des cibles dans chaque `target.namespace` (`mail`), lecture des sources namespacées dans chaque `source.namespaces`, Events (dont `default`, D20), ConfigMap et Lease dans le namespace de weebo-link |
| 0 | `ValidatingAdmissionPolicy` + `…Binding` | les hooks (en-têtes, cibles réservées) |
| 1 | Deployment `weebo-link` | le contrôleur (1 réplique active, lease du kit) |

Le RBAC est dérivé des règles par le chart de weebo-link lui-même, pas par
le `clusterRole` de `weebo-operator-lib` (D20).

### Donner une boîte à un utilisateur

```yaml
# weebo-si:2.argo/helm/auth/sub/values.yaml
identity:
  users:
    items:
      - crName: alice
        username: alice
        email: alice@weebo.poc
        isActive: true
        groupRefs: [weebo_user]
        links: [stalwart]          # rendu en annotation link.weebo.io/stalwart: enabled
```

weebo-link crée :

```yaml
apiVersion: stalwart.weebo.io/v1alpha1
kind: StalwartAccount
metadata:
  name: alice
  namespace: mail
  labels:
    app.kubernetes.io/managed-by: weebo-link
    link.weebo.io/link: authentik-user-stalwart-account
    link.weebo.io/state: Linked               # Linked | Detaching | Detached
  annotations:
    link.weebo.io/source: authentik.weebo.io/AuthentikUser/alice   # source cluster-scoped : pas de namespace (D11)
    link.weebo.io/source-uid: 6f1c…           # l'UID, pour ne pas confondre avec une source recréée
spec:
  domainRef: { name: weebo.poc }
  name: alice                  # spec.username de la source
  aliases: []                  # local-part de l'email s'il diffère du username (D15)
  state: Active                # isActive → Active, sinon Locked
  deletionPolicy: Orphan
```

```console
$ kubectl -n mail get stalwartaccount -l link.weebo.io/link -L link.weebo.io/state
NAME    INSTANCE   ADDRESS           READY   REASON       AGE   STATE
alice   default    alice@weebo.poc   True    Reconciled   20s   Linked
```

Pour l'utilisateur réel de weebo-si, `username: batleforc` et
`email: max@weebo.poc` (`weebo-si:2.argo/helm/auth/sub/values.yaml`) : la
boîte est `batleforc@weebo.poc`, avec l'alias `max`, pour que les mails
envoyés à l'adresse connue d'Authentik et des apps arrivent (D15).

### Retirer un utilisateur

`auth-app` est en `prune: false` (`weebo-si:2.argo/helm/auth/main/templates/authentik/sub-app.yaml`,
commentaire : supprimer un CR détruit l'objet Authentik, ce n'est jamais
l'effet d'un retrait de Git). Le départ se fait donc ainsi (D12) :

1. **Un commit : `isActive: false`.** weebo-authentik désactive le compte
   Authentik (plus de login OIDC). weebo-link voit la source changer et
   applique `state: Locked` : la boîte est verrouillée, App Passwords
   compris (RFC 0001, *Verrouillage*). La cible reste `Linked`.
2. **Plus tard, si l'on veut supprimer le compte Authentik :** retirer
   l'utilisateur des values, puis supprimer l'`AuthentikUser` à la main
   (`kubectl delete authentikuser <crName>`). weebo-link voit la cible sans
   source : il applique `onRemoval` (`spec.state: Locked`, déjà le cas) et
   le label `state: Detaching`.
3. Quand l'expression `safe` de la règle est vraie (`Locked`, `Ready` à
   jour), il pose `state: Detached`. La boîte reste dans Stalwart,
   verrouillée avec ses mails, et son CR reste visible.

Retirer l'utilisateur des values **sans** l'étape 1 ne fait rien : le CR
reste, l'utilisateur reste actif, la boîte reste `Linked`. Retirer
seulement `stalwart` de ses `links` passe directement à l'étape 2 côté
boîte (`Detaching`, puis `Detached`), le compte Authentik restant actif.

```console
$ kubectl -n mail get stalwartaccount -l link.weebo.io/state=Detached
NAME    INSTANCE   ADDRESS           READY   REASON       AGE
alice   default    alice@weebo.poc   True    Reconciled   41d
```

Si l'en-tête revient sur **la même** source (même UID), weebo-link
rattache le CR détaché tout seul. Si l'`AuthentikUser` a été supprimé puis
recréé (UID différent), le rattachement est explicite : l'en-tête vaut
`reattach` au lieu d'`enabled` (D14). Supprimer définitivement une boîte
reste un geste explicite : on libère la cible
(`link.weebo.io/release: requested`, D13), puis on passe sa
`deletionPolicy` à `Delete` et on supprime le CR.

## Design

### Vocabulaire

| Terme | Sens |
| --- | --- |
| **Source** | L'objet qui porte l'en-tête (`AuthentikUser`, cluster-scoped). Écrit dans Git, possédé par son opérateur. weebo-link n'y écrit jamais. |
| **Cible** | Le CR fabriqué (`StalwartAccount`). Écrit par weebo-link, réconcilié par son opérateur. |
| **Lien** | Une règle : kind source, kind cible, mapping, état sûr. |
| **En-tête** | Une annotation ou un label `link.weebo.io/*`. |
| **État sûr** | L'état de la cible qu'on peut laisser sans source, sans dommage. Défini par la brique cible. |
| **Libération** | Le retrait de tous les en-têtes d'une cible par weebo-link : elle redevient un CR ordinaire (D13). |

### En-têtes

| En-tête | Posé sur | Par | Sens |
| --- | --- | --- | --- |
| annotation `link.weebo.io/<header>: enabled \| reattach` | source | Git (chart de la source) | opt-in au lien dont le `header` vaut `<header>` (`stalwart`). `reattach` dit en plus : « rattache la cible `Detached` de ce nom, même si elle venait d'une autre source » (D14). Aucune autre valeur n'est admise : il n'y a pas de paramètre. |
| label `link.weebo.io/link: <lien>` | cible | weebo-link | la règle qui possède ce CR. C'est le sélecteur du contrôleur, des hooks et du panneau. |
| annotation `link.weebo.io/source` | cible | weebo-link | la source attendue, qui permet de retrouver un départ manqué : `<groupe>/<Kind>/<nom>` pour une source cluster-scoped, `<groupe>/<Kind>/<ns>/<nom>` pour une source namespacée (D11) |
| annotation `link.weebo.io/source-uid` | cible | weebo-link | l'UID de la source liée. Une source supprimée puis recréée sous le même nom n'est pas une continuité : son rattachement est explicite (D14). |
| label `link.weebo.io/state: Linked \| Detaching \| Detached` | cible | weebo-link | où en est le lien (label pour les sélecteurs) |
| annotation `link.weebo.io/release: requested` | cible | un humain | demande de libération (D13). Seul en-tête qu'un humain peut poser sur une cible liée ; weebo-link le retire avec les autres. |
| label `app.kubernetes.io/managed-by: weebo-link` | cible | weebo-link | convention |

Le domaine `link.weebo.io` ne recoupe pas celui des annotations du kit
(`<produit>.weebo.io/…`). Les opérateurs ignorent les annotations qu'ils ne
connaissent pas, donc un en-tête sur un `AuthentikUser` ne change rien pour
weebo-authentik.

### Une règle de lien

Les values du chart `link` sont rendues telles quelles dans la ConfigMap
`weebo-link-rules`, et relues à chaud :

```yaml
links:
  - name: authentik-user-stalwart-account
    header: stalwart                      # link.weebo.io/stalwart: enabled
    source:
      apiVersion: authentik.weebo.io/v1alpha1
      kind: AuthentikUser                 # cluster-scoped : pas de `namespaces`
      # namespaces: [team-a]              # seulement pour un kind source namespacé
    target:
      apiVersion: stalwart.weebo.io/v1alpha1
      kind: StalwartAccount
      namespace: mail
      name: { cel: source.metadata.name }
      spec:                               # littéral, ou {cel: …} évalué sur la source
        domainRef: { name: weebo.poc }
        name: { cel: source.spec.username }
        aliases: { cel: >-
          source.spec.email.endsWith('@weebo.poc') &&
          source.spec.email != source.spec.username + '@weebo.poc'
            ? [source.spec.email.split('@')[0]] : [] }
        state: { cel: 'source.spec.isActive ? "Active" : "Locked"' }
        deletionPolicy: Orphan
      immutable: [spec.name]              # changement → ImmutableFieldChangeNotAllowed, jamais de renommage
    onRemoval:
      patch:                              # surcharge du mapping complet (D16), pas un apply partiel
        spec: { state: Locked }
      safe: { cel: >-
        target.spec.state == "Locked" && has(target.status) &&
        has(target.status.observedGeneration) &&
        target.status.observedGeneration == target.metadata.generation &&
        has(target.status.conditions) &&
        target.status.conditions.exists(c, c.type == "Ready" && c.status == "True") }
      then: Keep                          # Keep (Detached) | Delete
```

- Une feuille est soit un littéral YAML, soit `{cel: "<expr>"}` évalué
  avec `source` (et `target` pour `safe`). Pas de boucle, pas d'appel
  externe : de la lecture de champs, des opérateurs, le ternaire et les
  fonctions de chaîne des extensions Kubernetes (`split`, `endsWith`…,
  D4).
- `source.spec.isActive` se lit sans `has()` : le schéma de
  `AuthentikUser` lui donne `default: true`, donc l'apiserver le pose
  toujours. `email` et `username` sont `required`.
- Une expression qui **échoue** à l'évaluation (champ absent sans `has()`)
  ne vaut jamais `true` : un mapping en erreur n'est pas appliqué, et un
  `safe` en erreur laisse la cible en `Detaching`. Dans les deux cas, un
  Event et une métrique sont émis (l'enregistreur du kit regroupe les
  Events identiques en une série). C'est vérifié : sans `has(target.status)`,
  une cible jamais réconciliée fait échouer `safe` au lieu de le rendre
  faux. D'où aussi `has()` sur `observedGeneration` et `conditions`, que
  le `Status` du kit peut omettre, pour qu'une cible fraîche donne `false`
  et non une erreur à chaque passage.
- Les entiers JSON sont des `int` CEL, comme dans l'apiserver
  (`json_to_cel` de `kube-cel`). Avec la conversion serde par défaut de la
  crate `cel`, `generation == 3` vaut `false` sans erreur (*Vérifications*).
- La règle est validée au chargement : apiVersion, kind **et scope**
  résolus par discovery (`namespaces` refusé sur un kind cluster-scoped,
  exigé sur un kind namespacé), expressions CEL compilées, champs
  `immutable` présents dans le mapping. Une règle invalide est refusée
  seule, avec un Event et une métrique. Les autres continuent.
- **Rediscovery :** une règle refusée parce que son kind est inconnu (CRD
  installé après weebo-link) est retentée à chaque resync, sans attendre un
  changement de la ConfigMap. Une règle refusée pour une autre raison (CEL,
  `immutable`) attend une nouvelle version des values.
- Une règle **retirée** des values ne déclenche aucun départ : ses cibles
  gardent leur dernier état, et une métrique compte les cibles au label
  `link` inconnu. Retirer une règle n'est jamais un départ en masse. Ses
  en-têtes sur les sources restent admis tant qu'ils ne changent pas
  (`link-headers`, D18), et ses cibles se libèrent par
  `link.weebo.io/release` (D13).

### Cycle de vie d'une cible

| Situation | Action de weebo-link |
| --- | --- |
| Source annotée, pas de cible | server-side apply du mapping complet (fieldManager `weebo-link/<lien>`, D16, D17), état `Linked` ; `record_write(Created)` |
| Source modifiée, ou resync | les champs du mapping sont comparés à la cible. Aucun ne diffère : aucune écriture. Sinon : apply du mapping complet, `record_write(Drift)` ; Event `TargetUpdated` si le champ venait de weebo-link (la source a changé), `DriftCorrected` si son dernier écrivain dans `managedFields` n'est pas `weebo-link/<lien>` (quelqu'un d'autre a changé la valeur) |
| Champ `immutable` modifié côté source | rien n'est appliqué ; Event `ImmutableFieldChangeNotAllowed` sur la source ; métrique. Un renommage passe par un nouveau `crName`, donc une nouvelle source et un nouveau CR (RFC 0001, *Ce que la brique de liaison consomme*) |
| Une cible du même nom existe, **sans** label `link` ou avec celui d'un autre lien | jamais d'adoption (règle du kit, D9) : Event `TargetAlreadyExists` sur la source |
| Source supprimée, désannotée, ou UID différent | apply des valeurs actuelles des champs du mapping, lues sur la cible, surchargées par `onRemoval.patch` (D16) ; état `Detaching` |
| `Detaching` et `safe` vrai | `then: Keep` → état `Detached` ; `then: Delete` → suppression (la `deletionPolicy` de la cible s'applique), `record_write(Deleted)` |
| `Detaching`, la même source revient (même UID, annotée) | retour en `Linked` : apply du mapping complet. L'état suit alors `isActive` |
| `Detaching`, une source d'UID différent apparaît | le départ continue jusqu'à `Detached` ; le rattachement ne se fait qu'ensuite, et seulement par `reattach` (D14) |
| `Detached`, la même source est réannotée (même UID) | rattachement automatique : c'est le même objet |
| `Detached`, source d'UID différent, en-tête `enabled` | rien ; Event `ReattachRequired` sur la source |
| `Detached`, source d'UID différent, en-tête `reattach` | rattachement : nouveau `source-uid`, apply du mapping complet, `Linked` ; Event `TargetReattached` sur la cible |
| `link.weebo.io/release: requested` sur une cible `Detached`, ou dont la règle n'existe plus | libération : tous les en-têtes `link.weebo.io/*` et `managed-by` retirés, Event `TargetReleased`. Le CR est un CR ordinaire, hors des hooks |
| `release` sur une cible `Linked` ou `Detaching` | ignorée, Event `ReleaseRefused` : il faut d'abord retirer l'en-tête de la source |

Le contrôleur n'a **aucun finalizer sur les sources**. Il ne retient la
suppression d'aucun objet d'un autre opérateur. Les départs manqués
pendant une panne sont retrouvés au redémarrage et à chaque resync (5 min
par défaut, comme `STEADY_REQUEUE_SECS` du kit) : une cible dont la source
n'existe plus est traitée comme un départ.

Chaque écriture d'une cible incrémente sa `metadata.generation` et
déclenche donc un reconcile de weebo-stalwart. C'est la raison de
« comparer avant d'appliquer » : au repos, un resync n'écrit rien, et
`weebo_link_remote_writes_total{cause="drift"}` reste à zéro.

### Les hooks

Les hooks sont des `ValidatingAdmissionPolicy`, évaluées par l'apiserver
lui-même : pas de webhook, pas de `failurePolicy` qui s'ouvre quand un pod
tombe.

**`link-headers`** : sur les kinds source des règles, un en-tête
`link.weebo.io/*` **ajouté ou modifié** ne vaut que `enabled` ou
`reattach`, et seulement pour un `header` connu. Un en-tête inchangé par
rapport à `oldObject` passe toujours (D18) : sans cela, un en-tête d'une
règle retirée, ou une valeur posée pendant que le binding était en
`Audit`, rendrait la source non modifiable, y compris le retrait de son
finalizer par weebo-authentik, et bloquerait sa suppression.

```yaml
apiVersion: admissionregistration.k8s.io/v1
kind: ValidatingAdmissionPolicy
metadata:
  name: link-headers
spec:
  failurePolicy: Fail
  matchConstraints:
    resourceRules:
      - apiGroups: [authentik.weebo.io]      # rendu depuis les sources des règles
        apiVersions: ["*"]
        resources: [authentikusers]
        operations: [CREATE, UPDATE]
  variables:
    - name: old
      expression: >-
        request.operation == 'UPDATE' && has(oldObject.metadata.annotations)
          ? oldObject.metadata.annotations : {}
  validations:
    - expression: >-
        !has(object.metadata.annotations) ||
        object.metadata.annotations.all(k,
          !k.startsWith('link.weebo.io/') ||
          (k in variables.old && variables.old[k] == object.metadata.annotations[k]) ||
          (k.substring(14) in ['stalwart'] &&
           object.metadata.annotations[k] in ['enabled', 'reattach']))
      messageExpression: "'en-tête link.weebo.io/* inconnu, ou valeur autre que enabled/reattach (RFC 0002)'"
```

**`link-targets`** : sur les kinds cibles, seul weebo-link crée un CR qui
porte un en-tête `link.weebo.io/*`, et seul lui change sa spec ou ces
en-têtes. Une exception : un humain peut **ajouter seulement**
`link.weebo.io/release: requested`, spec inchangée (D13). Tout le reste
reste ouvert à tous : les finalizers et le status de l'opérateur cible,
les annotations du kit (`force-orphan`, `rotate-secret`, `import-pending`)
posées par un humain ou par l'importer, et la suppression. Les en-têtes
sont comparés comme des ensembles `clé=valeur`, labels et annotations
confondus, parce que l'ordre d'itération d'une map CEL n'est pas garanti.

```yaml
apiVersion: admissionregistration.k8s.io/v1
kind: ValidatingAdmissionPolicy
metadata:
  name: link-targets
spec:
  failurePolicy: Fail
  matchConstraints:
    resourceRules:
      - apiGroups: [stalwart.weebo.io]
        apiVersions: ["*"]
        resources: [stalwartaccounts]
        operations: [CREATE, UPDATE]
  variables:
    - name: newHeaders
      expression: >-
        (has(object.metadata.labels)
          ? object.metadata.labels.filter(k, k.startsWith('link.weebo.io/')).map(k, 'l:' + k + '=' + object.metadata.labels[k])
          : []) +
        (has(object.metadata.annotations)
          ? object.metadata.annotations.filter(k, k.startsWith('link.weebo.io/')).map(k, 'a:' + k + '=' + object.metadata.annotations[k])
          : [])
    - name: oldHeaders
      expression: >-
        request.operation != 'UPDATE' ? [] :
        (has(oldObject.metadata.labels)
          ? oldObject.metadata.labels.filter(k, k.startsWith('link.weebo.io/')).map(k, 'l:' + k + '=' + oldObject.metadata.labels[k])
          : []) +
        (has(oldObject.metadata.annotations)
          ? oldObject.metadata.annotations.filter(k, k.startsWith('link.weebo.io/')).map(k, 'a:' + k + '=' + oldObject.metadata.annotations[k])
          : [])
    - name: linked
      expression: "size(variables.newHeaders) > 0 || size(variables.oldHeaders) > 0"
    - name: isLink   # rendu par le chart : namespace de la release et nom du ServiceAccount
      expression: >-
        request.userInfo.username ==
        'system:serviceaccount:{{ .Release.Namespace }}:{{ include "weebo-operator.serviceAccountName" . }}'
    - name: sameSpec
      expression: >-
        request.operation == 'UPDATE' &&
        (has(object.spec) == has(oldObject.spec)) &&
        (!has(object.spec) || object.spec == oldObject.spec)
    - name: sameHeaders
      expression: >-
        variables.newHeaders.all(h, h in variables.oldHeaders) &&
        variables.oldHeaders.all(h, h in variables.newHeaders)
    - name: releaseOnly
      # Sur une cible déjà liée seulement : sans `size(oldHeaders) > 0`, un
      # humain pouvait poser `release` sur un CR non lié, qui devenait lié
      # (spec figée, `release` irretirable) sans que weebo-link le libère.
      expression: >-
        size(variables.oldHeaders) > 0 &&
        variables.oldHeaders.all(h, h in variables.newHeaders) &&
        variables.newHeaders.all(h, h in variables.oldHeaders ||
          h == 'a:link.weebo.io/release=requested')
  validations:
    - expression: >-
        variables.isLink || !variables.linked ||
        (variables.sameSpec && (variables.sameHeaders || variables.releaseOnly))
      messageExpression: "'ce CR appartient à un lien link.weebo.io : sa spec et ses en-têtes link.weebo.io/* sont écrits par weebo-link ; seul link.weebo.io/release: requested peut être ajouté (RFC 0002)'"
      reason: Forbidden
```

Chaque `has()` est nécessaire. Une première version comparait
`object.metadata.annotations == oldObject.metadata.annotations` sans garde.
Sur un CR sans annotations, l'expression plantait, et c'est
`failurePolicy: Fail` qui refusait à sa place, pour la mauvaise raison.

Le chart rend les `resourceRules`, la liste des `header` et l'identité de
weebo-link (`isLink`) depuis les règles et ses propres values : rien n'est
écrit en dur. Une règle retirée dont le kind cible n'est plus visé par
aucune autre règle sort des `resourceRules` : ses cibles redeviennent
modifiables par tous, en gardant leurs en-têtes comme trace. Les deux
bindings sont en `validationActions: [Deny]` (`[Audit]` pendant le
rollout).

### Décisions

Les décisions marquées **proposée (à valider)** sont des choix de
l'auteur à confirmer ; l'alternative écartée est donnée en une ligne.

| # | Décision | Pourquoi |
| --- | --- | --- |
| D1 | Un contrôleur dédié, `weebo-link`, et non Kyverno | La brique `hook` peut disparaître. Un `generate` Kyverno supprime aussi la cible au départ de la source, alors qu'il faut la verrouiller. weebo-link fait « patcher puis garder », et retrouve un départ manqué grâce aux en-têtes de la cible. |
| D2 | Les règles vivent dans la configuration de weebo-link, sans CRD | Peu de liens, tous définis par les admins, tous dans Git. Un CRD `Link` ajouterait un objet par lien, un schéma et un status pour la même information. Repli si des équipes veulent leurs propres liens (*Travaux futurs*). |
| D3 | L'opt-in est un en-tête sans paramètre sur la source | C'est la convention `inject-certs: enabled` de weebo-si (`weebo-si:2.argo/helm/hook/sub/templates/mount-cert.yaml`), et l'auteur de la source décide dans Git. Tout le reste (namespace, domaine, nom) est figé dans la règle : un en-tête ne dit que oui (`enabled`), oui et rattacher (`reattach`, D14), ou rien. |
| D4 | Mapping en CEL, limité à la lecture, au ternaire et aux fonctions des extensions Kubernetes, évalué par `kube-cel` | C'est le langage de l'apiserver, des VAP et des `x-kubernetes-validations`. `kube-cel` 0.8 (kube-rs, sur `cel` 0.13) ajoute au cœur de CEL les extensions de Kubernetes (`substring`, chaînes, listes, ensembles…) et sa conversion JSON. Pas de templating Helm à double échappement. |
| D5 | Départ = patch vers l'état sûr, la cible est gardée (`Detached`) | Une boîte ne se perd pas par un retrait de values. C'est réversible, et la RFC 0001 (D8) le demande. La suppression reste un `then: Delete` explicite, par règle. |
| D6 | Aucun finalizer ni aucune écriture sur la source | weebo-link n'interfère jamais avec un objet d'un autre opérateur, ni avec Argo CD. Les en-têtes de la cible suffisent à retrouver un départ. |
| D7 | Server-side apply, fieldManager par lien | weebo-link ne possède que les champs de son mapping. Les finalizers et le status de l'opérateur cible ne sont jamais en conflit. Comment il applique : D16, D17. |
| D8 | Hooks en `ValidatingAdmissionPolicy`, pas de webhook | Évaluées dans l'apiserver : elles tiennent quand weebo-link est arrêté, sans `failurePolicy` qui s'ouvre et sans certificat à gérer. |
| D9 | Jamais d'adoption d'une cible existante | C'est la règle du kit. Une cible sans label `link` reste à son auteur : `TargetAlreadyExists`. Pour la passer sous un lien, il faut la migration explicite de *Exploitation* (suppression en `Orphan`, recréation par weebo-link, import). |
| D10 | Sur le runtime du kit, sans le template opérateur | Ce n'est pas un opérateur produit : ni instance, ni gateway distante, ni CRD, ni Condition. Il reprend du kit : `leader` (`LeaseLock`, `acquire_leadership`, `register_leader_gauge`), `health` (`Health`, `routes`), `telemetry::init`, `shutdown::Shutdown`, `pod::own_namespace`, `metrics::ReconcileMetrics` (`record`, `record_write`), `events::{announce_change, announce_drift, announce_error, spawn_error}` (sur un `ObjectReference`, donc sur `DynamicObject`), `weebo-kit-domain` (`Reason`, `reason_codes!`, `drift::RemoteWrite`), `cog`, `checks.yml`/`publish.yml` et le chart `weebo-operator-lib` sauf son RBAC (D20). Il ne reprend pas `events::outcome_events`/`announce_outcome` (fondés sur les `Condition`), `status`, `conditions`, `admission`, `webhook_server`, ni `controller`/`WatchScope`, typés (`DynamicType = ()`) : le contrôleur sur types dynamiques reste dans weebo-link, à verser au kit si un second utilisateur apparaît (règle des deux opérateurs). `Drift` et `record_write` prennent des `&'static str` (champ, kind) : les chemins et kinds des règles sont internés au chargement (ensemble borné par les règles), sans changement du kit. |
| D11 | **Proposée (à valider).** Format de `link.weebo.io/source` : `<groupe>/<Kind>/<nom>` pour une source cluster-scoped, `<groupe>/<Kind>/<ns>/<nom>` pour une source namespacée | Le scope vient de la discovery, donc le nombre de segments est sans ambiguïté, et la valeur se lit comme un chemin d'API. Alternative : toujours quatre segments, namespace vide (`…/AuthentikUser//alice`). |
| D12 | **Proposée (à valider).** Départ d'un utilisateur = `isActive: false` (ou retrait de l'en-tête pour ne retirer que la boîte) ; `prune: false` d'`auth-app` inchangé ; la suppression d'un `AuthentikUser` reste manuelle | `prune: false` est voulu (`sub-app.yaml`) : un retrait de Git ne détruit aucun objet Authentik. `isActive: false` coupe le login et verrouille la boîte en un seul commit, et se défait par un commit. Alternative : `prune: true` sur `auth-app`, contraire à son commentaire et valable pour tous ses CR (groupes, applications, providers). |
| D13 | **Proposée (à valider).** Sortie d'une cible liée par `link.weebo.io/release: requested`, seul en-tête qu'un humain peut ajouter ; weebo-link libère une cible `Detached` ou sans règle en retirant tous ses en-têtes | Sans sortie, `link-targets` refuse « passer `deletionPolicy` à `Delete` puis supprimer », et les cibles d'une règle retirée restent figées sans propriétaire. Une libération demandée garde l'invariant « seul weebo-link change les en-têtes », et ne peut pas libérer une cible encore `Linked`. Alternative : exempter `spec.deletionPolicy` de `sameSpec`, plus simple mais qui laisse les cibles figées pour tout le reste. |
| D14 | **Proposée (à valider), bloquante.** Rattachement automatique seulement pour la même source (même UID) ; une source d'UID différent n'est rattachée que si son en-tête vaut `reattach` | Une source recréée par quelqu'un d'autre ne doit pas déverrouiller l'ancienne boîte, ses mails compris. `reattach` est écrit dans Git par qui écrit la source, comme `enabled`. Alternative : rattachement automatique au retour d'une source du même nom (brouillon précédent, Q4). |
| D15 | **Proposée (à valider).** Identité de la boîte : `spec.name` = `spec.username` (la JIT OIDC retombe sur ce compte, RFC 0001) ; la local-part de `spec.email`, si elle est dans le domaine de la règle et diffère du username, devient un alias | L'utilisateur réel a `username: batleforc` et `email: max@weebo.poc` : sans alias, la boîte serait `batleforc@` alors qu'Authentik et les apps écrivent à `max@`. L'alias passe par la `StalwartNamespacePolicy` et la revendication unique (RFC 0001, D16) comme tout alias. Alternatives : imposer `email == username@domaine` par une validation de la règle (refus + Event, et corriger l'email de batleforc) ; ou dériver `spec.name` de l'email, ce qui casse la correspondance JIT. |
| D16 | Chaque apply porte le **mapping complet** sous le même fieldManager, après comparaison | Un apply partiel `{spec: {state: Locked}}` sous le fieldManager qui possédait tout le mapping lui retirerait les autres champs (SSA : un champ absent d'un apply est abandonné, et supprimé s'il n'a pas d'autre propriétaire). Au départ, la source n'existe plus : weebo-link relit sur la cible les valeurs actuelles des champs du mapping et les surcharge par `onRemoval.patch`. La comparaison avant écriture suit la RFC 0003 (`record_write`, `DriftCorrected`). |
| D17 | **Proposée (à valider).** Apply avec `force: true` | `link-targets` fait de weebo-link le seul écrivain de la spec d'une cible liée. Un conflit SSA ne vient donc que d'un contournement (binding en `Audit`, importer qui change une valeur) ou d'un SSA forcé de l'importer, qui a pris la propriété d'un champ : weebo-link la reprend, et l'Event `DriftCorrected` le dit. Alternative : sans `force`, un conflit donne un Event `TargetConflict` et la cible attend un humain. |
| D18 | `link-headers` ne valide que les en-têtes ajoutés ou modifiés par rapport à `oldObject` | Un en-tête d'une règle retirée, ou posé pendant un `Audit`, ne doit pas rendre la source immodifiable, ni bloquer le retrait du finalizer de weebo-authentik. Une règle retirée ne bloque donc rien ; seule une nouvelle valeur est contrôlée. |
| D19 | weebo-link ne pose jamais `import-pending` | La RFC 0001 lui laisse la question. Poser `import-pending` sur un `StalwartObjectAlreadyExists` serait une adoption par nom par un autre chemin (D9) : relier un compte JIT existant reste un geste humain, par `weebo-stalwart-importer` et sa garde `RemoteObjectClaimed`. |
| D20 | **Proposée (à valider).** RBAC rendu par le chart de weebo-link, hors du `weebo-operator.clusterRole` de la lib | Ce template accorde `secrets` get/create/update/patch/delete à l'échelle du cluster sauf `watch.namespace` + `clusterSecrets: false`, ce qui ne convient pas à des sources cluster-scoped, et ses `kindRules` supposent un seul `params.group` avec status et finalizers (`charts/weebo-operator-lib/templates/_rbac.tpl`). weebo-link inclut les autres templates de la lib (ServiceAccount, Deployment, lease, métriques) et rend : un ClusterRole de lecture des seuls kinds source cluster-scoped, un Role par namespace cible et par namespace source, Events dans ces namespaces et dans `default` (où vont les Events d'un objet cluster-scoped). Alternative : une évolution du kit (`params.secrets: false`, règles par groupe), une release mineure (0.x) pour un seul utilisateur. |
| D21 | Codes de raison propres à weebo-link, dans son catalogue `reason_codes!` | Le kit n'en définit aucun. Codes au style du kit : `Linked`, `TargetUpdated`, `DriftCorrected`, `TargetAlreadyExists`, `ImmutableFieldChangeNotAllowed`, `ReattachRequired`, `TargetReattached`, `TargetDetaching`, `TargetDetached`, `TargetReleased`, `ReleaseRefused`, `RuleInvalid`, `MappingFailed`, `SafeEvaluationFailed`. Ils alimentent la page « codes de raison » de la doc (*Clôture*). |

### Architecture

Un dépôt Rust `weebo-link`, en hexagonal comme les opérateurs :

| Crate | Contenu |
| --- | --- |
| `domain` | `Rule`, `Mapping` (littéral ou CEL), états `Linked`/`Detaching`/`Detached`, la décision pure « que faire de cette paire source/cible », la comparaison des champs du mapping, le catalogue `reason_codes!` |
| `application` | `reconcile_source`, `reconcile_target`, `sweep` (resync complet et rediscovery) ; ports `Sources`, `Targets`, `Rules` |
| `adapters-inbound` | un watcher `DynamicObject` par kind source et par kind cible (filtré sur le label `link`), rechargement de la ConfigMap |
| `adapters-outbound` | server-side apply (`force: true`) et patch sur `Api<DynamicObject>`, discovery, Events |
| `operator` | le binaire : lease, santé, OTLP, métriques du kit (D10) |
| `testkit` | envtest du kit avec deux CRD factices (source cluster-scoped et cible namespacée), sans les vrais opérateurs |

Le contrôleur est sans état. La vérité est dans les sources (Git), dans
les en-têtes et les champs des cibles et dans les règles.

### Contrat d'une brique cible

Pour être la cible d'un lien, une brique publie dans sa RFC ou dans la doc
de son opérateur :

1. le kind cible et les champs qu'un lien peut renseigner ;
2. son **état sûr** : ce qu'il faut patcher, et l'expression `safe` qui le
   confirme. Pour `StalwartAccount` : `state: Locked`, puis `Ready` à jour
   (RFC 0001, D20). Une cible sans données propres dit « aucun » et peut
   prendre `then: Delete` ;
3. une `deletionPolicy` qui ne détruit rien par défaut.

L'opérateur cible n'a aucun code à ajouter, et ne sait pas qu'il est lié.

### Ce qui change dans weebo-si

- **`weebo-si:2.argo/helm/auth/sub/templates/identity/users.yaml`** rend `links` (et `reattach`) en en-têtes :

  ```yaml
  {{- $u := . }}
  metadata:
    name: {{ .crName }}
    annotations:
      argocd.argoproj.io/sync-wave: "4"
  {{- range .links }}
      link.weebo.io/{{ . }}: {{ ternary "reattach" "enabled" (has . ($u.reattach | default list)) }}
  {{- end }}
  ```

- **`weebo-si:2.argo/helm/link`** : le chart (Application imbriquée vers le chart OCI
  `weebo-link`, comme les opérateurs), les values `links:`, les VAP.
- **Argo CD** : rien à ignorer. Les cibles ne sont dans aucune Application,
  donc elles ne sont ni suivies ni prunées. Les en-têtes de la source sont
  dans Git. `auth-app` reste en `prune: false` (D12).
- **`weebo-si:2.argo/helm/mail`** (RFC 0001) : les `StalwartAccount` d'utilisateurs
  qui y sont déclarés à la main en sortent à la migration (Phase 3).

## Sécurité

- **Qui crée un lien :** qui peut écrire la source. `AuthentikUser` est
  **cluster-scoped** : seul un droit cluster-wide sur `authentikusers`
  l'accorde, c'est-à-dire le dépôt weebo-si et l'Application `auth-app`
  (Argo CD), ou un admin du cluster. Ces droits donnent déjà la main sur le
  compte Authentik lui-même. L'en-tête n'a pas de paramètre : il ne peut
  viser ni un autre namespace, ni un autre domaine, ni un autre nom que
  ceux de la règle. `reattach` relève du même droit : qui peut recréer
  l'`AuthentikUser` peut déjà ouvrir une session OIDC sous son nom ; c'est
  le verrou de la boîte, `Locked` tant que personne n'écrit `reattach`
  dans Git, qui protège ses mails (D14).
- **Qui définit une règle :** qui peut écrire les values de
  `weebo-si:2.argo/helm/link`, donc les admins de la plateforme.
- **Privilèges Kubernetes de weebo-link**, rendus depuis les règles (D20) :
  - `get/list/watch` sur les kinds source : en **ClusterRole** pour un kind
    cluster-scoped (`authentikusers`, inévitable), en Role dans chaque
    `source.namespaces` pour un kind namespacé ;
  - `get/list/watch/create/patch` (SSA : `create` quand l'apply crée la
    cible) sur les kinds cibles, en **Role dans
    chaque `target.namespace`** (`mail`), et `delete` si une règle a
    `then: Delete` ;
  - Events (`create/patch`) dans les namespaces cibles et sources, et dans
    `default` pour les sources cluster-scoped ;
  - lecture de sa ConfigMap et Lease, dans son namespace.

  Il n'a aucun droit sur les Secrets, ni sur les CRD qui ne sont pas dans
  une règle. Ce n'est vrai que parce que son chart n'utilise pas le
  ClusterRole de `weebo-operator-lib`, qui accorde les Secrets (D20). Il
  lit en entier les `AuthentikUser` (username, email, groupes) ; il n'en
  utilise que les champs du mapping.
- **Côté produit :** les cibles passent par l'admission de l'opérateur
  cible et par sa `NamespacePolicy`, comme pour n'importe quel auteur.
  weebo-link n'a aucun jeton produit.
- **Rayon d'impact :**
  - un weebo-link compromis peut créer, verrouiller ou déverrouiller des
    comptes dans `mail`, et poser des alias bornés par la
    `StalwartNamespacePolicy`. Il ne peut ni lire de mail, ni toucher aux
    comptes d'envoi des apps (autres namespaces, et label `link` absent),
    ni écrire dans `auth` ;
  - **son identité contourne `link-targets`** : quiconque peut créer un Pod
    sous son ServiceAccount, ou un jeton pour lui
    (`serviceaccounts/token`), dans le namespace de weebo-link, écrit les
    cibles liées comme lui. Ce namespace ne porte que weebo-link, et seuls
    Argo CD et les admins du cluster y ont `pods`/`serviceaccounts/token`
    en écriture.
- **Contournements :**
  - *weebo-link arrêté :* pas de création ni de verrouillage, mais les VAP
    tiennent. Au retour, le resync rattrape tout. Pendant la panne, un user
    passé `isActive: false` ne peut déjà plus se connecter par OIDC, mais
    ses App Passwords restent valides (RFC 0001, *Exploitation*).
  - *Un CR cible écrit à la main avec un faux label `link` :* refusé par
    `link-targets`.
  - *Une source recréée sous le même nom par quelqu'un d'autre :*
    l'UID diffère, donc l'ancienne cible passe en `Detached` (verrouillée)
    et y reste. Seul un en-tête `reattach`, écrit dans Git, la rattache
    (D14).
  - *Compte JIT créé avant le lien :* la cible échoue en
    `StalwartObjectAlreadyExists` (RFC 0001, D9). La reprise passe par
    `weebo-stalwart-importer` ; weebo-link ne pose pas `import-pending`
    (D19).
- **Données :** aucun secret recopié ni lu.

## Exploitation

- **Mode d'échec :** voir les contournements. weebo-link n'est sur le
  chemin d'aucun Pod ni d'aucune admission. Arrêté, il ne bloque rien ; il
  retarde seulement les créations et les verrouillages.
- **Rollout :**
  1. `users.yaml` rend `links` (aucun effet tant que weebo-link n'est pas
     là).
  2. weebo-link avec la règle, VAP en `Audit`, `links: [stalwart]` sur un
     utilisateur de test.
  3. Test de bout en bout : création, `isActive: false` (verrouillage),
     `isActive: true`, retrait de l'en-tête (`Detached`), retour de
     l'en-tête (rattachement automatique), suppression manuelle puis
     recréation de l'`AuthentikUser` (`ReattachRequired`, puis `reattach`),
     libération (`release`), départ pendant que weebo-link est arrêté (le
     resync doit verrouiller), retrait de la règle (aucun départ, source
     toujours modifiable).
  4. VAP en `Deny`.
  5. Migration des `StalwartAccount` déclarés à la main (ci-dessous), puis
     `links: [stalwart]` sur tous les utilisateurs qui ont une boîte.
- **Rollback :** couper l'Application `link`. Les cibles restent, dans leur
  dernier état, comme des CR ordinaires. Les VAP partent avec le chart.
- **Migrer un `StalwartAccount` déclaré à la main** (RFC 0001, phase 2,
  `weebo-si:2.argo/helm/mail`) : weebo-link ne l'adopte jamais
  (`TargetAlreadyExists`, D9), et poser ses en-têtes à la main serait
  défait par l'auto-sync de l'Application `mail` et refusé par
  `link-targets`. Par compte :
  1. vérifier que le mapping donne la même spec (`state` contre
     `isActive`, sinon la boîte se déverrouille ; `quota` et les champs
     hors mapping sont perdus du CR, à ajouter au mapping en littéral ou à
     accepter : `None` est « pas d'avis », la valeur reste dans Stalwart) ;
  2. retirer le CR du chart `mail` et le supprimer : `deletionPolicy:
     Orphan`, le compte Stalwart reste intact ;
  3. `links: [stalwart]` sur l'utilisateur : weebo-link crée la cible, qui
     échoue en `StalwartObjectAlreadyExists` ;
  4. `weebo-stalwart-importer` la relie (`import-pending`, garde
     `RemoteObjectClaimed`). Il réapplique la même spec, donc il passe
     `link-targets` sans `Audit`.

  Entre 2 et 4, le compte n'est piloté par aucun CR, mais Stalwart le sert
  dans son dernier état.
- **Importer un compte JIT sous une cible liée** (`StalwartObjectAlreadyExists`) :
  lancer `weebo-stalwart-importer`. S'il réapplique exactement la spec de
  weebo-link, il passe ; sinon, passer le binding `link-targets` en
  `Audit` le temps de l'import, puis repasser en `Deny`. weebo-link
  reprend ensuite ses champs (D17).
- **Observabilité :**
  - `kubectl get <cible> -A -l link.weebo.io/link -L link.weebo.io/state` ;
  - les Events sur la source (`TargetAlreadyExists`,
    `ImmutableFieldChangeNotAllowed`, `ReattachRequired`) et sur la cible
    (`TargetUpdated`, `DriftCorrected`, `TargetDetached`, `TargetReleased`…,
    D21) ;
  - les métriques préfixées `weebo_link` : `weebo_link_targets{link,state}`
    (gauge), `weebo_link_rule_errors_total{link}`,
    `weebo_link_unowned_targets` (gauge, label `link` inconnu), celles de
    `ReconcileMetrics` du kit (`…_reconcile_duration_seconds`,
    `…_remote_writes_total{kind,cause}`) et le gauge leader du kit, vers
    otel-cluster ;
  - alertes souhaitées : une cible `Detaching` depuis plus de 10 min (état
    sûr jamais atteint), une règle en erreur, pas de leader, des écritures
    `cause="drift"` répétées. weebo-si n'a pas de moteur d'alerte (RFC
    0001, D22) : le mécanisme suit la RFC 0001, Q11. Le chart les fournira
    en `PrometheusRule` (`weebo-operator.prometheusRule` de la lib) si Q11
    retient Prometheus.
- **Montée de version :** weebo-link suit les tags du kit, comme les
  opérateurs. Une nouvelle version d'un CRD source ou cible ne change que
  les règles, qui sont validées par discovery au chargement et à chaque
  resync.

## Alternatives considérées

1. **Kyverno `generate`** (brouillon initial de cette RFC). Il n'y a pas de
   code à écrire. Mais il dépend de la brique `hook`, qui peut disparaître,
   et il supprime la cible au départ de la source. Il faut alors une garde
   qui refuse le départ et impose deux commits. Écarté.
2. **Un CRD `Link`** par lien, lu par weebo-link. Il apporte un status par
   lien et une délégation possible aux équipes, au prix d'un objet de plus
   par lien. Ce n'est pas nécessaire tant que les liens sont l'affaire des
   admins (D2), et la porte reste ouverte.
3. **Un kind « source » dans chaque opérateur cible**
   (`StalwartAccountSource`, brouillon de la RFC 0001). Écarté par la RFC
   0001, D7.
4. **Metacontroller (`DecoratorController`).** Il fonctionne par hooks
   HTTP et annotations, donc il est proche dans l'esprit. Mais c'est une
   dépendance tierce de plus, et ses enfants sont liés par ownerRef dans le
   namespace de la source : la cible ne peut pas être dans `mail`, et un
   départ la supprime par le garbage collector.
5. **kro / Crossplane Compositions.** Ils créent un CRD par composition,
   l'inverse de « pas d'objet en plus », plus un contrôleur tiers.
6. **Générer les cibles dans Helm**, avec le chart `mail` qui lirait la
   liste des users de `weebo-si:2.argo/helm/auth/sub`. Il n'y a aucun moteur, mais deux charts
   partagent les mêmes values. Surtout, au départ d'un user, Argo CD
   prunerait la cible au lieu de la verrouiller.
7. **Un webhook mutant ou validant servi par weebo-link** au lieu des VAP.
   Il est plus expressif, mais c'est un serveur de plus sur le chemin de
   l'admission. En `Fail` il bloque quand weebo-link tombe, en `Ignore` il
   s'ouvre. Les VAP font la même chose dans l'apiserver.

## Inconvénients et risques

- **Un composant de plus à maintenir.** Il reste petit : pas d'API produit,
  pas de secret, pas de CRD. Le gros du code est la lecture de règles et un
  apply générique, testés en envtest sur des CRD factices.
- **Le contrôleur dynamique n'est pas dans le kit** (D10), ni son RBAC
  (D20). C'est un écart assumé, à remonter dans le kit dès qu'un second
  utilisateur apparaît.
- **Une lecture cluster-wide des `AuthentikUser`.** Inévitable pour une
  source cluster-scoped : weebo-link voit les username et email de tous
  les utilisateurs.
- **Le ServiceAccount de weebo-link est une clé.** Il contourne
  `link-targets` ; son namespace doit rester fermé (*Sécurité*).
- **Des cibles `Detached` s'accumulent.** C'est voulu, puisque ce sont des
  boîtes conservées. Le nettoyage est un geste humain (libération, puis
  `Delete`), aidé par le sélecteur `link.weebo.io/state=Detached`.
- **Le départ dépend d'un geste dans les values** (`isActive: false`) :
  retirer un utilisateur de Git sans ce geste ne verrouille rien (D12).
- **CEL dans les values Helm.** Les erreurs apparaissent au chargement de
  la règle, pas au `helm template`. Le chart lance un `weebo-link
  check-rules` en test Helm, et la CI des charts aussi. La feature
  `validation` de `kube-cel` fournit un `VapEvaluator` qui permettrait aussi
  à `check-rules` de tester les VAP sans apiserver. Elle n'est pas encore
  essayée.
- **`kube-cel` 0.8 épingle `cel` 0.13**, alors que `cel` est en 0.15 :
  rien de connu à ce jour, mais `task audit` (cargo-deny) doit surveiller
  cette version retenue, et une montée suivra celle de `kube-cel`.

## Questions ouvertes

| # | Question | Bloquant |
| --- | --- | --- |
| Q1 | ~~Quelle crate CEL en Rust ?~~ **Tranchée :** `kube-cel` 0.8 (sur `cel` 0.13), avec la conversion `json_to_cel` (entiers en `int`). | ~~oui~~ |
| Q2 | ~~La VAP `link-targets` laisse-t-elle passer les écritures légitimes ?~~ **Tranchée** pour la forme sans `releaseOnly` : elle ne compare que la spec et les en-têtes `link.weebo.io/*`, et passe les 18 scénarios. Les formes ci-dessus (D13, D18) sont rejouées (Q7). | ~~oui~~ |
| Q3 | ~~Kubernetes du cluster ?~~ **Tranchée :** v1.36.2 (`weebo-si:1.pulu-*/Pulumi.dev.yaml`). Les VAP et leurs `variables` sont GA. | ~~non~~ |
| Q4 | ~~Rattachement automatique ou explicite ?~~ **Proposée :** automatique pour la même source, `reattach` pour une source recréée (D14, à valider). | oui |
| Q5 | Faut-il une règle `AuthentikGroup` → `StalwartGroup` dès la v0.1.0, ou attendre le premier besoin ? | non |
| Q6 | Où publier les règles en doc : une page générée depuis les values de `weebo-si:2.argo/helm/link` ? | non |
| Q7 | ~~Les formes révisées de `link-headers` (variable `old`, en-têtes inchangés admis, `reattach`) et de `link-targets` (`releaseOnly`, `isLink` rendu) passent-elles leurs scénarios sur le kube-apiserver v1.36.2 ? Nouveaux cas : retrait du finalizer avec un en-tête d'une règle retirée, valeur inchangée posée en `Audit`, valeur modifiée refusée, `release` seul admis, `release` avec un changement de spec refusé.~~ **Tranchée (2026-10-19)** sur kube-apiserver v1.36.2 + etcd 3.5.32, RBAC, en impersonation : 73 scénarios, tous conformes (en-tête d'une règle retirée toléré, retrait du finalizer et suppression jusqu'au bout, valeur invalide refusée, humain refusé sur la spec d'une cible liée, `release` seul admis, SSA forcé de weebo-link admis). Un bug trouvé et corrigé ci-dessus : `releaseOnly` admettait `release` sur un CR non lié, qui se retrouvait figé ; avec `size(oldHeaders) > 0` le cas est refusé, les 72 autres inchangés. Restent à décider : un humain ne peut pas retirer un `release` qu'il a posé (il reste jusqu'à la libération) ; `[Warn, Audit]` plutôt que `[Audit]` au rollout (en `Audit` seul, rien n'est visible sans backend d'audit) ; `reason: Forbidden` sur `link-headers` pour un 403 comme `link-targets` (422 aujourd'hui). | ~~oui~~ |
| Q8 | ~~`split` (ou `indexOf`) est-il dans `Context::default().with_all()` de `kube-cel` 0.8, pour l'alias de D15 ? Sinon, l'alias passe par `substring` et une autre fonction, ou D15 retombe sur son alternative.~~ **Tranchée :** oui. `kube-cel` 0.8.0 (`cel` 0.13.0) enregistre `split`, `substring`, `indexOf`, `lowerAscii`, `join` dans `with_all()` (`src/functions/strings.rs:18-24`, feature `strings` par défaut) ; même résultat sur l'apiserver v1.36.2 (`batleforc` / `max@weebo.poc` → `aliases=[max]`). D15 garde `split`. | ~~non~~ |
| Q9 | ~~`weebo-crd-docs` sans CRD : il lit `crdDir` sans test d'existence. Un dossier vide suffit-il pour n'avoir que le site et les pages de référence, ou faut-il une option du package ?~~ **Tranchée :** un dossier vide suffit pour le site (index CRD vide), un dossier absent échoue (`ENOENT`) ; mais les pages de référence échouent sans CRD (`generate.mjs:272`, « the CRDs span 0 API groups »), et `generateReference` n'est pas exporté. Il faut une option du package, par exemple `reference.group` explicite quand il n'y a pas de CRD (release mineure du kit). | ~~non~~ |

### Vérifications (2026-10-18)

**Q2 et `link-headers`.** Un kube-apiserver v1.36.2 local (la version du
cluster), avec etcd et des CRD factices `StalwartAccount` et
`AuthentikUser`. Chaque scénario est joué en impersonation, sous le compte
de weebo-link, celui de l'opérateur, un humain ou l'importer. Ces
résultats portent sur les formes **d'avant la revue** ; les formes
actuelles sont à rejouer (Q7).

| Scénario | Attendu | Première forme | Forme vérifiée |
| --- | --- | --- | --- |
| weebo-link crée une cible liée ; change `spec.state` | OK | OK | OK |
| un humain crée une cible au label `link` | refus | refus | refus |
| l'opérateur pose son finalizer ; écrit `status` (sous-ressource) | OK | OK | OK |
| un humain pose `force-orphan` ou `rotate-secret` | OK | **refus** | OK |
| l'importer pose puis retire `import-pending` | OK | **refus** | OK |
| l'importer fait un SSA forcé avec une spec identique | OK | OK | OK |
| un humain change `spec.state` | refus | refus | refus |
| un humain retire `link.weebo.io/link`, change `state` ou `source` | refus | refus | refus |
| un humain crée une cible sans en-tête | OK | OK | OK |
| un humain pose le label `link` sur ce CR | refus | refus (par erreur d'évaluation) | refus |
| un humain pose seulement l'annotation `link.weebo.io/source` | refus | **OK** | refus |
| un humain supprime une cible liée | OK | OK | OK |

`link-headers`, dans sa forme d'avant la revue, passait ses 7 cas : sans
annotation, `enabled`, valeur `"true"` refusée, en-tête inconnu refusé,
autres annotations, status écrit par weebo-authentik, retrait de l'en-tête.
Elle ne comparait pas à `oldObject` : un en-tête inconnu déjà présent
bloquait toute écriture de la source (D18).

Conséquence pour l'importer : il est accepté sur une cible liée tant qu'il
réapplique **la même spec**. Avec une spec différente, il est refusé. Pour
importer un compte JIT sous une cible liée avec une autre spec, on passe la
VAP en `Audit` le temps de l'import (*Exploitation*).

**Q1.** Un binaire de test sur `cel` 0.15, puis sur `kube-cel` 0.8 / `cel`
0.13, appliqué aux expressions de la règle et à des objets JSON réalistes :

- **Mapping et `safe` :** le mapping (`metadata.name`, `spec.username`,
  le ternaire sur `isActive`, une annotation à clé avec points) et `safe`
  donnent les valeurs attendues. `safe` est faux si `observedGeneration`
  est en retard. L'alias de D15 (`split`) n'a pas été essayé (Q8).
- **Macros :** `all`, `exists` sur les clés d'une map, `filter().map()` et
  `has()` fonctionnent.
- **`substring` :** absent du cœur de `cel`, présent avec
  `Context::default().with_all()` de `kube-cel`.
- **Erreurs :** une erreur de syntaxe donne la position et ce qui était
  attendu. Un champ absent donne « No such key: … », une variable inconnue
  « Undeclared reference ». D'où la règle « erreur ≠ vrai » et les `has()`
  dans `safe`.
- **Piège des entiers :** avec la conversion serde par défaut de `cel`
  0.13, un entier JSON positif devient `uint`, et `generation == 3` vaut
  `false` sans erreur, alors que `>` fonctionne. Avec `json_to_cel`
  (entiers en `int`, comme l'apiserver), c'est `true`. Entre deux champs
  JSON, l'égalité marche dans les deux cas.

**Revue (2026-10-18), lu dans les dépôts :** `AuthentikUser` est
`scope: Cluster` et `isActive` a `default: true`
(`weebo-authentik:deploy/crd/authentikuser.yaml`) ; `auth-app` est en
`prune: false` (`weebo-si:2.argo/helm/auth/main/templates/authentik/sub-app.yaml`) ;
weebo-authentik 0.18.3 déployé, sur le kit v0.18.0 ; le RBAC de
`weebo-operator-lib` (`_rbac.tpl`) et les signatures de `events.rs` et
`metrics.rs` du kit v0.18.0 (D10, D20).

## Travaux futurs

- **D'autres paires**, une fois leurs CRD prêts : `AuthentikGroup` →
  `StalwartGroup` (sans état sûr, `then: Delete`), `AuthentikUser` →
  `ForgejoUser` (pré-création avant le premier login OAuth), puis Che et
  BatleHub. Chacune avec son contrat de cible.
- **Un CRD `Link` namespacé** (alternative 2), si des équipes doivent
  définir leurs propres liens dans leur namespace.
- **Le contrôleur dynamique et son RBAC versés au kit**
  (`weebo-kit-runtime`, `weebo-operator-lib`), si un second composant en a
  l'usage.

## Plan d'implémentation

Chaque case est une PR qu'on peut merger seule.

**Spike (avant `Accepted`)**

- [x] Choix de la crate CEL (Q1) ; VAP `link-targets` contre les
      écritures de l'opérateur, des humains et de l'importer (Q2) ; version
      du cluster (Q3). Voir *Vérifications*.
- [ ] Formes révisées des VAP rejouées (Q7) ; `split` dans `kube-cel`
      (Q8) ; D11–D15, D17 et D20 validées

**Phase 1 — weebo-link v0.1.0**

- [ ] Dépôt sur les crates runtime du kit v0.18.0 (D10), `cog.toml`,
      `checks.yml`/`publish.yml`, chart sur `weebo-operator-lib` avec son
      propre RBAC (D20), catalogue `reason_codes!` (D21)
- [ ] Règles : chargement, validation par discovery (scope compris),
      rediscovery au resync, compilation CEL, rechargement à chaud,
      `check-rules`
- [ ] Apply des cibles (SSA du mapping complet, `force`, fieldManager par
      lien), comparaison avant écriture, `record_write`, `DriftCorrected`,
      `immutable`, `TargetAlreadyExists`
- [ ] Départ : `onRemoval` surchargeant les valeurs actuelles, `safe`,
      `Detaching`/`Detached`, resync des cibles orphelines, rattachement
      (même UID, `reattach`), libération (`release`)
- [ ] VAP `link-headers` et `link-targets` rendues par le chart depuis les
      règles et le ServiceAccount
- [ ] envtest sur CRD factices (source cluster-scoped) : création, mise à
      jour, aucune écriture au repos, départ, départ pendant une panne,
      source recréée (UID, `reattach`), libération, règle retirée, faux
      label refusé

**Phase 2 — weebo-si, premier lien (après weebo-stalwart v0.2.0, RFC 0001)**

- [ ] `weebo-si:2.argo/helm/auth/sub` : `links` et `reattach` rendus en en-têtes dans `users.yaml`
- [ ] `weebo-si:2.argo/helm/link` : Application, règle
      `authentik-user-stalwart-account`, VAP en `Audit`
- [ ] Un utilisateur de test, scénario de bout en bout (*Rollout*, étape
      3 : départ par `isActive: false`, pas par un retrait de values),
      VAP en `Deny`
- [ ] Panneau « Liens » (états, cibles sans règle, écritures) ; alertes
      après la RFC 0001, Q11

**Phase 3 — weebo-si, migration et généralisation**

- [ ] Migration de chaque `StalwartAccount` d'utilisateur déclaré à la
      main dans `weebo-si:2.argo/helm/mail` (*Exploitation*) : retrait du
      chart en `Orphan`, `links: [stalwart]`, import
- [ ] Tous les utilisateurs avec boîte en `links: [stalwart]`, dont
      batleforc et son alias `max` (D15)

**Clôture**

- [ ] RFC 0001 et left-todo à jour, doc de weebo-link sur
      `@weebo/operator-docs` : `defineSite`, pages de référence (codes de
      raison, Events, métriques) et une page des règles ; pas de pages CRD,
      puisqu'il n'y en a pas (Q9). RFC passée en `Implemented`

## Références

- [ValidatingAdmissionPolicy](https://kubernetes.io/docs/reference/access-authn-authz/validating-admission-policy/)
- [CEL dans Kubernetes](https://kubernetes.io/docs/reference/using-api/cel/)
- [Server-side apply et field managers](https://kubernetes.io/docs/reference/using-api/server-side-apply/)
- weebo-operator-kit v0.18.0 : `crates/weebo-kit-runtime` (`leader.rs`,
  `health.rs`, `telemetry.rs`, `metrics.rs`, `events.rs` ; `controller.rs`
  et `scope.rs` typés), `crates/weebo-kit-domain` (`drift.rs`,
  `reason.rs`), `charts/weebo-operator-lib/templates/_rbac.tpl`,
  `docs/plan.md` (« Ce qui va dans le kit », règle des deux opérateurs,
  jamais d'adoption par nom), `packages/operator-docs`
- weebo-authentik 0.18.3 : `deploy/crd/authentikuser.yaml`, `crates/api/src/user.rs`
- weebo-si : `2.argo/helm/hook/sub/templates/mount-cert.yaml` (convention
  d'opt-in par annotation), `2.argo/helm/auth/sub/templates/identity/users.yaml`,
  `2.argo/helm/auth/sub/values.yaml`, `2.argo/helm/auth/main/templates/authentik/sub-app.yaml`
- [RFC 0001](rfc-0001-stalwart.md) : D7, D8, D9, D20, D22, Q11, *Ce que la brique de liaison consomme*, *Verrouillage*
- RFC 0003 : lire, comparer, n'écrire que la dérive

## Changelog

| Date | Changement |
| --- | --- |
| 2026-10-18 | Brouillon initial (Kyverno `generate` + garde de départ) |
| 2026-10-18 | Kyverno abandonné, car la brique `hook` peut disparaître : contrôleur `weebo-link` sans CRD (règles dans ses values), départ en un commit par patch vers l'état sûr et cible gardée `Detached`, hooks en `ValidatingAdmissionPolicy` natives |
| 2026-10-18 | Q1–Q3 tranchées (*Vérifications*) : `kube-cel` 0.8 et sa conversion JSON en `int` ; `link-targets` réécrite pour ne comparer que la spec et les en-têtes `link.weebo.io/*` (18/18 scénarios sur un kube-apiserver v1.36.2) ; `has(target.status)` dans `safe`, et une évaluation en erreur ne vaut jamais vrai |
| 2026-10-18 | RFC déplacée dans weebo-operator-kit |
| 2026-10-18 | Revue appliquée : base kit v0.18.0 et weebo-authentik 0.18.3 ; `AuthentikUser` cluster-scoped (plus de `namespaces`, format de `source` D11, RBAC D20) ; `auth-app` en `prune: false`, départ par `isActive: false` (D12) ; `link-headers` ne valide que ce qui change (D18) ; sortie par `link.weebo.io/release` (D13) et ServiceAccount rendu dans `link-targets` ; rattachement explicite d'une source recréée (D14) et transitions depuis `Detaching` ; alias depuis l'email (D15) ; SSA du mapping complet, comparaison avant écriture, `force` (D16, D17) ; `import-pending` jamais posé (D19) ; codes au style du kit (D21) ; `has()` dans `safe` ; rediscovery ; Events dans `default` ; alertes renvoyées à la RFC 0001, Q11 ; métriques renommées ; phase de migration ; Q4 proposée, Q7–Q9 ajoutées |
| 2026-10-19 | Q7, Q8, Q9 vérifiées : VAP rejouées sur kube-apiserver v1.36.2 (73 scénarios ; `releaseOnly` corrigé), `split` présent dans `kube-cel` 0.8, référence de docs impossible sans CRD sans option du package |
