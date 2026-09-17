// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0
import type {SidebarsConfig} from '@docusaurus/plugin-content-docs';

const sidebars: SidebarsConfig = {
  docs: [
    'intro',
    {
      type: 'category',
      label: 'Getting Started',
      collapsed: false,
      items: ['docs/quickstart'],
    },
    {
      type: 'category',
      label: 'User Guide',
      collapsed: false,
      items: [
        'docs/user-guide',
        'docs/modalities',
        'docs/strategic-benchmarking',
        'docs/prompt-library',
        'docs/results-publication',
        'docs/platforms',
      ],
    },
    {
      type: 'category',
      label: 'Methodology',
      collapsed: true,
      items: ['docs/performance-methodology'],
    },
    {
      type: 'category',
      label: 'Reference',
      collapsed: true,
      items: [
        'docs/feature-reference',
        'docs/cli-reference',
        'docs/output-schema',
        'docs/comparison',
        'docs/limitations',
        'docs/release-notes',
      ],
    },
  ],
};

export default sidebars;
