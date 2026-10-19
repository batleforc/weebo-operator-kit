import type { Metadata } from 'next';
import './global.css';
import { JetBrains_Mono, Silkscreen } from 'next/font/google';
import { basePath, DocsRootProvider, siteMetadata } from '@weebo/operator-docs/site';
import { site } from '@/lib/shared';

// The BatleHub pairing: a square-pixel display face for identity and scale,
// a mono text face for every fact. next/font self-hosts both at build time;
// the theme reaches them through these two CSS variables.
const silkscreen = Silkscreen({
  subsets: ['latin'],
  weight: ['400', '700'],
  variable: '--font-silkscreen',
  display: 'swap',
});

const jetbrainsMono = JetBrains_Mono({
  subsets: ['latin'],
  variable: '--font-jetbrains-mono',
  display: 'swap',
});

// Absolute OG image URLs on the published site (origin + base path, as
// Next resolves image paths under it); unset locally.
export const metadata: Metadata = siteMetadata(
  site,
  process.env.DOCS_SITE_URL ? new URL(`${basePath}/`, process.env.DOCS_SITE_URL) : undefined,
);

export default function Layout({ children }: LayoutProps<'/'>) {
  return (
    <html
      lang="en"
      className={`${silkscreen.variable} ${jetbrainsMono.variable} font-sans`}
      suppressHydrationWarning
    >
      <body className="flex flex-col min-h-screen">
        <DocsRootProvider>{children}</DocsRootProvider>
      </body>
    </html>
  );
}
