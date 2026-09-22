// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0
import fs from 'fs';
import path from 'path';
import type {Config} from '@docusaurus/types';
import type {Options as PresetOptions} from '@docusaurus/preset-classic';

/** Default docs stamp from root Cargo.toml `version` (override with DOCS_VERSION). */
function cargoPackageVersion(): string {
  const cargoToml = fs.readFileSync(
    path.join(__dirname, '..', 'Cargo.toml'),
    'utf8'
  );
  const match = cargoToml.match(/^version\s*=\s*"([^"]+)"/m);
  return match ? `v${match[1]}` : 'dev';
}

function normalizeBaseUrl(base: string): string {
  return base.endsWith('/') ? base : `${base}/`;
}

const docsVersion = process.env.DOCS_VERSION || cargoPackageVersion();
const docsBaseUrl = normalizeBaseUrl(
  process.env.DOCS_BASE_URL || '/metrum-ai-bench-cli/'
);
const docsVersionsUrl =
  process.env.DOCS_VERSIONS_URL || `${docsBaseUrl}versions.json`;

const config: Config = {
  title: 'Metrum AI Bench CLI Docs',
  tagline:
    'Load and performance measurement for OpenAI-compatible AI endpoints',
  favicon: 'img/favicon.ico',

  // Published at https://docs.metrum.ai/metrum-ai-bench-cli/
  url: 'https://docs.metrum.ai',
  baseUrl: docsBaseUrl,
  trailingSlash: false,

  organizationName: 'metrum-ai',
  projectName: 'bench-cli',
  customFields: {
    docsBaseUrl,
    docsVersion,
    docsVersionsUrl,
  },

  onBrokenLinks: 'throw',
  markdown: {
    hooks: {
      onBrokenMarkdownLinks: 'warn',
    },
  },

  i18n: {
    defaultLocale: 'en',
    locales: ['en'],
  },

  presets: [
    [
      'classic',
      {
        docs: {
          path: 'content',
          routeBasePath: '/',
          sidebarPath: './sidebars.ts',
          showLastUpdateAuthor: false,
          showLastUpdateTime: false,
        },
        blog: false,
        theme: {
          customCss: './src/css/custom.css',
        },
      } satisfies PresetOptions,
    ],
  ],

  plugins: [
    '@signalwire/docusaurus-plugin-llms-txt',
    [
      '@docusaurus/plugin-client-redirects',
      {
        // trailingSlash is false: one entry covers /docs and /docs/
        redirects: [{from: '/docs', to: '/docs/quickstart'}],
      },
    ],
  ],

  themeConfig: {
    // Social card: white product logo (platforms compose cards on dark chrome).
    image: 'img/metrum_ai_bench_white.svg',
    colorMode: {
      defaultMode: 'light',
      disableSwitch: false,
      respectPrefersColorScheme: false,
    },
    navbar: {
      // No `title`: the logo artwork includes the product name.
      logo: {
        alt: 'Metrum AI Bench CLI',
        src: 'img/metrum_ai_bench_black.svg',
        srcDark: 'img/metrum_ai_bench_white.svg',
      },
      items: [
        {to: '/', label: 'Home', position: 'left'},
        {to: '/docs/quickstart', label: 'Docs', position: 'left'},
        {
          href: 'https://github.com/metrum-ai/bench-cli',
          label: 'GitHub',
          position: 'right',
        },
      ],
    },
    footer: {
      style: 'light',
      links: [
        {
          title: 'Docs',
          items: [
            {label: 'Getting Started', to: '/docs/quickstart'},
            {label: 'CLI Reference', to: '/docs/cli-reference'},
            {label: 'Release Notes', to: '/docs/release-notes'},
          ],
        },
        {
          title: 'Product',
          items: [
            {
              label: 'GitHub Releases',
              href: 'https://github.com/metrum-ai/bench-cli/releases',
            },
            {
              label: 'Prompt library',
              href: 'https://huggingface.co/datasets/metrum-ai/prompt-library',
            },
          ],
        },
      ],
      copyright: `Copyright © ${new Date().getFullYear()} Metrum AI, Inc.`,
    },
    prism: {
      additionalLanguages: ['bash', 'json', 'yaml'],
    },
  },
};

export default config;
