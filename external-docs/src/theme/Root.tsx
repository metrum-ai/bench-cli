// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0
import React, {useEffect, type ReactNode} from 'react';
import useDocusaurusContext from '@docusaurus/useDocusaurusContext';

type VersionEntry = {
  label?: string;
  path?: string;
  version: string;
};

type VersionsManifest = {
  aliases?: VersionEntry[];
  latest?: string;
  versions?: VersionEntry[];
};

/** Reject open redirects: only same-origin relative paths (leading `/`, not `//`). */
function safePath(path: string): string | null {
  return path.startsWith('/') && !path.startsWith('//') ? path : null;
}

function normalizeBase(path: string): string {
  return path.endsWith('/') ? path : `${path}/`;
}

function targetForVersion(
  targetBase: string,
  currentBases: string[],
  location: Location
): string {
  const current = currentBases
    .map(normalizeBase)
    .sort((left, right) => right.length - left.length)
    .find((base) => location.pathname.startsWith(base));
  const target = normalizeBase(targetBase);
  const suffix = current ? location.pathname.slice(current.length) : '';
  return `${target}${suffix}${location.search}${location.hash}`;
}

function installVersionSelector(customFields: Record<string, unknown>) {
  const mount = document.querySelector('.navbar__items--right');
  if (!mount || document.querySelector('.metrum-docs-version-select')) {
    return;
  }

  const docsVersion = String(customFields.docsVersion || 'dev');
  const docsBaseUrl = normalizeBase(String(customFields.docsBaseUrl || '/'));
  const versionsUrl = String(
    customFields.docsVersionsUrl || `${docsBaseUrl}versions.json`
  );

  fetch(versionsUrl, {cache: 'no-store'})
    .then((response) => (response.ok ? response.json() : undefined))
    .catch(() => undefined)
    .then((manifest: VersionsManifest | undefined) => {
      const entries = manifest?.versions?.length
        ? manifest.versions
        : [{label: docsVersion, path: docsBaseUrl, version: docsVersion}];
      const aliases = manifest?.aliases || [];
      const options = [...aliases, ...entries];
      if (options.length < 1) {
        return;
      }
      const currentBases = [
        docsBaseUrl,
        ...options
          .filter((entry) => entry.version === docsVersion && entry.path)
          .map((entry) => String(entry.path)),
      ];

      const wrapper = document.createElement('div');
      wrapper.className = 'navbar__item metrum-docs-version-select';

      const select = document.createElement('select');
      select.setAttribute('aria-label', 'Documentation version');
      for (const entry of options) {
        const option = document.createElement('option');
        const fallbackPath = `${docsBaseUrl}${entry.version}/`;
        const candidate = entry.path || fallbackPath;
        const path = safePath(candidate);
        if (!path) {
          continue;
        }
        option.value = path;
        option.textContent = entry.label || entry.version;
        if (entry.version === docsVersion && entry.label !== 'latest') {
          option.selected = true;
        }
        select.appendChild(option);
      }
      if (select.options.length < 1) {
        return;
      }
      select.addEventListener('change', () => {
        const target = safePath(select.value);
        if (!target) {
          return;
        }
        window.location.href = targetForVersion(
          target,
          currentBases,
          window.location
        );
      });

      wrapper.appendChild(select);
      mount.appendChild(wrapper);
    });
}

export default function Root({children}: {children: ReactNode}): ReactNode {
  const {siteConfig} = useDocusaurusContext();

  useEffect(() => {
    installVersionSelector(siteConfig.customFields as Record<string, unknown>);
  }, [siteConfig.customFields]);

  return <>{children}</>;
}
