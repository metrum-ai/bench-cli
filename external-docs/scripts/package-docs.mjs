// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import path from 'node:path';

const options = Object.fromEntries(process.argv.slice(2).reduce((all, arg, i, args) => arg.startsWith('--') ? [...all, [arg.slice(2), args[i + 1]]] : all, []));
const version = options.version;
if (!version || !options['latest-version'] || !options['release-versions']) throw new Error('usage: package-docs.mjs --version V --latest-version V --release-versions "V ..." [--site-root slug] [--dist-dir dir] [--build-dir dir] [--latest-build-dir dir]');
const site = options['site-root'] ?? 'metrum-ai-bench-cli';
const dist = options['dist-dir'] ?? 'dist';
const build = options['build-dir'] ?? 'build';
let latestBuild = options['latest-build-dir'] || '';
if (!existsSync(build)) throw new Error(`ERROR: build dir ${build} does not exist. Run 'make build' first.`);
if (latestBuild && !existsSync(latestBuild)) { console.warn(`WARNING: latest-build-dir ${latestBuild} not found, falling back to ${build}`); latestBuild = ''; }
mkdirSync(dist, { recursive: true });
const staging = path.join(dist, 'staging');
rmSync(staging, { recursive: true, force: true });
const siteDir = path.join(staging, site);
const versions = [...new Set([...options['release-versions'].split(/\s+/).filter(Boolean), version])].sort().reverse();
const manifest = { generated_at: new Date().toISOString().replace(/\.\d{3}Z$/, 'Z'), latest: options['latest-version'], product: 'Metrum AI Bench CLI', versions: versions.map(v => ({ version: v, label: v, path: `/${site}/${v}/`, is_latest: v === options['latest-version'] })), aliases: [{ label: 'latest', path: `/${site}/latest/`, version: options['latest-version'] }] };
const sortedJson = value => Array.isArray(value) ? value.map(sortedJson) : value && typeof value === 'object' ? Object.fromEntries(Object.keys(value).sort().map(key => [key, sortedJson(value[key])])) : value;
const manifestJson = `${JSON.stringify(sortedJson(manifest), null, 2)}\n`;
mkdirSync(siteDir, { recursive: true });
cpSync(build, path.join(siteDir, version), { recursive: true });
if (latestBuild) cpSync(latestBuild, path.join(siteDir, 'latest'), { recursive: true });
writeFileSync(path.join(siteDir, 'versions.json'), manifestJson);
const archive = `${site}-docs-${version}.tar.gz`;
execFileSync('tar', ['-czf', path.join(dist, archive), '-C', staging, site]);
const digest = createHash('sha256').update(readFileSync(path.join(dist, archive))).digest('hex');
writeFileSync(path.join(dist, `${archive}.sha256`), `${digest}  ${archive}\n`);
writeFileSync(path.join(dist, `${site}-docs-versions.json`), manifestJson);
rmSync(staging, { recursive: true, force: true });
console.log(`Packaged: ${path.join(dist, archive)}\nChecksum: ${path.join(dist, `${archive}.sha256`)}\nVersions: ${path.join(dist, `${site}-docs-versions.json`)}\nSHA256:   ${digest}`);
