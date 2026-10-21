import catalog from '../../crd-catalog.config.mjs';

// The CRD IDE's schema directories, one per catalog entry (written by
// scripts/crd-schemas.mjs).
export const crdSources = [...catalog.operators, ...catalog.thirdParty].map(({ name }) => ({
  base: `/crd-schemas/${name}`,
}));
