import Link from 'next/link';
import { repoUrl } from '@weebo/operator-docs/site';
import pkg from '@weebo/operator-docs/package.json';
import { site } from '@/lib/shared';

// The package's OperatorHome is an operator's landing page ("Install the
// chart", an example CR): the kit has neither, so this page lays out the
// same proof sheet with the theme's classes and its own words.

// The workspace version: the package follows it (.hooks/bump-versions.sh).
const version = `v${pkg.version}`;

const features = [
  {
    title: 'Crates',
    body: 'Reason codes, drift detection, the allow-list engine, CRD field types, secret stores (Kubernetes, Vault), the controller skeleton, leader election, health, metrics, Events, admission and the envtest harness.',
    href: '/docs/architecture',
    link: 'Architecture',
  },
  {
    title: 'Tooling',
    body: 'A library chart, shared Taskfiles pinned by sha256, reusable checks and publish workflows, a cargo-generate template and the docs-site package — all moved together by task kit:sync.',
    href: '/docs/guides/new-operator',
    link: 'Start an operator',
  },
  {
    title: 'Operators',
    body: 'weebo-forgejo, weebo-authentik, weebo-batlehub, weebo-angos and weebo-stalwart build on it; ProxyAuthK8S takes its docs package and shared tasks.',
    href: '/docs/operators',
    link: 'Who uses the kit',
  },
];

export default function HomePage() {
  return (
    <div className="flex flex-1 flex-col px-4 sm:px-6">
      <section className="mx-auto flex w-full max-w-[1100px] flex-col gap-6 pt-10 pb-10 sm:pt-16">
        <span className="sheet-label">Shared core of the weebo operators · {version}</span>
        <h1 className="sheet-display">{site.name}</h1>
        <p className="sheet-caption">
          The plumbing every weebo Kubernetes operator needs around its own business logic — in
          the same hexagonal layout the operators use — plus the chart, tasks, CI, template and
          docs site they would otherwise copy around.
        </p>
        <div className="flex flex-wrap items-center gap-3">
          <Link href="/docs" className="action">
            Read the docs
          </Link>
          <Link href="/docs/operators" className="ctl">
            Operators using it
          </Link>
          <a href={repoUrl(site)} className="ctl" rel="noreferrer">
            View on GitHub
          </a>
        </div>
      </section>

      <section className="sheet-region mx-auto w-full max-w-[1100px]">
        <div className="grid gap-6 md:grid-cols-2">
          <div className="flex min-w-0 flex-col gap-3">
            <h2 className="sheet-label">1 · Start an operator</h2>
            <pre className="sheet-pre">
              <code>{`cargo generate \\
  --git https://github.com/batleforc/weebo-operator-kit \\
  --tag ${version} templates/operator`}</code>
            </pre>
          </div>
          <div className="flex min-w-0 flex-col gap-3">
            <h2 className="sheet-label">2 · Move to a new release</h2>
            <pre className="sheet-pre">
              <code>{`task kit:sync TAG=${version}`}</code>
            </pre>
          </div>
        </div>
      </section>

      <section className="sheet-region mx-auto grid w-full max-w-[1100px] md:grid-cols-3">
        {features.map((f) => (
          <div key={f.title} className="sheet-feature flex flex-col gap-3 py-6 md:px-6 md:py-0 md:first:ps-0">
            <h2>{f.title}</h2>
            <p className="text-[13px] leading-[1.6] text-fd-muted-foreground">{f.body}</p>
            <Link href={f.href}>{f.link}</Link>
          </div>
        ))}
      </section>
    </div>
  );
}
