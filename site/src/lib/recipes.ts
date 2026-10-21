import type { Recipe } from '@weebo/operator-docs/crd-ide';

// The CRD IDE's starting points that span operators (content/docs/ide.mdx).
// `<…>` marks what to fill in; the IDE lists each one until it is.

const FORGEJO_FROM_STORES = `apiVersion: external-secrets.io/v1
kind: ExternalSecret
metadata:
  name: forgejo-token
spec:
  secretStoreRef: { kind: ClusterSecretStore, name: "<store>" }
  data:
    - secretKey: token
      remoteRef: { key: "<path of the Forgejo admin token>" }
---
apiVersion: cert-manager.io/v1
kind: Certificate
metadata:
  name: forgejo-ca
spec:
  secretName: forgejo-ca
  commonName: forgejo-ca
  issuerRef: { kind: ClusterIssuer, name: "<issuer that signs Forgejo's certificate>" }
---
apiVersion: forgejo.weebo.io/v1alpha1
kind: ForgejoLocalInstance
metadata:
  name: forgejo
spec:
  url: "<https://git.example.com>"
  tokenSecretRef: { name: forgejo-token, key: token }
  tls:
    caSecretRef: { name: forgejo-ca, key: ca.crt }
---
apiVersion: forgejo.weebo.io/v1alpha1
kind: ForgejoOrganization
metadata:
  name: platform
spec:
  name: platform
  instanceRef: { kind: LocalInstance, name: forgejo }
`;

const ANGOS_SSO = `apiVersion: authentik.weebo.io/v1alpha1
kind: AuthentikLocalInstance
metadata:
  name: authentik
spec:
  url: "<https://auth.example.com>"
  tokenSecretRef: { name: "<Secret holding the Authentik API token>", key: token }
---
apiVersion: authentik.weebo.io/v1alpha1
kind: AuthentikApplication
metadata:
  name: angos
spec:
  instanceRef: { kind: LocalInstance, name: authentik }
  name: Angos
  slug: angos
  provider:
    kind: oauth2
    oauth2:
      clientType: public
      authorizationFlow: "<authorization flow slug>"
      invalidationFlow: "<invalidation flow slug>"
      allowedRedirectUris:
        - { matchingMode: strict, url: "<https://registry.example.com/auth/callback>" }
  secretTargets:
    - name: angos-oidc
---
apiVersion: angos.weebo.io/v1alpha1
kind: AngosLocalInstance
metadata:
  name: angos
spec:
  storage:
    fs: {}
  auth:
    oidc:
      - name: authentik
        issuerFrom: { name: angos-oidc, key: AUTHENTIK_URL }
  ui:
    enabled: true
    oidc:
      provider: authentik
      clientIdFrom: { name: angos-oidc, key: AUTHENTIK_CLIENT_ID }
`;

export const RECIPES: Recipe[] = [
  {
    id: 'forgejo-from-stores',
    label: 'Forgejo: token from a secret store, CA from cert-manager',
    description:
      'An ExternalSecret writes the admin token, a cert-manager Certificate the CA; the local instance reads both, and an organization lives on it.',
    yaml: FORGEJO_FROM_STORES,
  },
  {
    id: 'angos-sso',
    label: 'Angos UI sign-in through Authentik',
    description:
      "An Authentik OAuth2 application writes its client id and URL to a Secret; Angos's OIDC provider and UI read them from it.",
    yaml: ANGOS_SSO,
  },
];
