// The CRDs the kit's CRD IDE offers: every weebo operator's, at a pinned
// tag, and a few third-party ones operators are used with, at a pinned
// release. `task site:catalog` downloads them into crd-catalog/ (committed:
// the build stays offline and a bump is a reviewable diff); the build turns
// them into public/crd-schemas/<name>/ (scripts/crd-schemas.mjs).
//
// An operator's links come from its own deploy/crd/links.json, which its
// crdgen writes from kit v0.23 on: until it is released with it, its kinds
// are listed without links. Third-party CRDs carry theirs here (`links`,
// weebo-crd-docs' CrdLinks).

/** @type {{ operators: { name: string, repo: string, tag: string }[], thirdParty: { name: string, url: string, kinds: string[], links?: import('@weebo/operator-docs/crd-docs').CrdLinks }[] }} */
const catalog = {
  operators: [
    { name: 'forgejo', repo: 'batleforc/weebo-forgejo', tag: 'v0.7.0' },
    { name: 'authentik', repo: 'batleforc/weebo-authentik', tag: 'v0.21.0' },
    { name: 'angos', repo: 'batleforc/weebo-angos', tag: 'v0.3.0' },
    { name: 'batlehub', repo: 'batleforc/weebo-batlehub', tag: 'v0.5.0' },
    { name: 'stalwart', repo: 'batleforc/weebo-stalwart', tag: 'v0.3.0' },
    // Not a weebo operator, but on the kit's docs and api crate (its crdgen).
    { name: 'proxyauthk8s', repo: 'batleforc/ProxyAuthK8S', tag: 'v0.3.0' },
  ],
  thirdParty: [
    {
      name: 'external-secrets',
      url: 'https://raw.githubusercontent.com/external-secrets/external-secrets/v2.12.0/deploy/crds/bundle.yaml',
      kinds: ['ExternalSecret', 'SecretStore', 'ClusterSecretStore'],
      links: {
        ExternalSecret: {
          references: [
            {
              path: 'spec.secretStoreRef',
              to: [
                { group: 'external-secrets.io', kind: 'SecretStore' },
                { group: 'external-secrets.io', kind: 'ClusterSecretStore' },
              ],
            },
          ],
          produces: [
            {
              kind: { kind: 'Secret' },
              name: ['spec.target.name', 'metadata.name'],
              keys: 'spec.data[].secretKey',
            },
          ],
        },
      },
    },
    {
      name: 'cert-manager',
      url: 'https://github.com/cert-manager/cert-manager/releases/download/v1.21.2/cert-manager.crds.yaml',
      kinds: ['Certificate', 'Issuer', 'ClusterIssuer'],
      links: {
        Certificate: {
          references: [
            {
              path: 'spec.issuerRef',
              to: [
                { group: 'cert-manager.io', kind: 'Issuer' },
                { group: 'cert-manager.io', kind: 'ClusterIssuer' },
              ],
            },
          ],
          produces: [{ kind: { kind: 'Secret' }, name: ['spec.secretName'], keyNames: ['tls.crt', 'tls.key', 'ca.crt'] }],
        },
      },
    },
  ],
};

export default catalog;
