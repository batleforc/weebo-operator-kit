import { defineSite } from '@weebo/operator-docs/site';

// What makes this site the kit's; everything else comes from
// @weebo/operator-docs, installed from this very tree (package.json).
export const site = defineSite({
  name: 'weebo-operator-kit',
  description:
    'The shared core of the weebo Kubernetes operators: Rust crates, a library chart, shared tasks, reusable CI, an operator template and the docs-site package.',
  git: {
    user: 'batleforc',
    repo: 'weebo-operator-kit',
    branch: 'main',
    contentDir: 'site/content/docs',
  },
});
