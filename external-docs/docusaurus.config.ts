import type {Config} from '@docusaurus/types';
import type {Options as PresetOptions} from '@docusaurus/preset-classic';

const docsVersion = process.env.DOCS_VERSION || 'v1.0.0-rc.6';
const docsBaseUrl = process.env.DOCS_BASE_URL || '/metrum-ai-bench-cli/';
const docsVersionsUrl =
  process.env.DOCS_VERSIONS_URL || '/metrum-ai-bench-cli/versions.json';

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

  plugins: ['@signalwire/docusaurus-plugin-llms-txt'],

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
