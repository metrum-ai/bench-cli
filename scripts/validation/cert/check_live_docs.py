#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Check links within the published documentation tree; never follow external sites."""
import argparse
from html.parser import HTMLParser
from collections import deque
import sys
from urllib.error import HTTPError, URLError
from urllib.parse import urldefrag, urljoin, urlsplit
from urllib.request import HTTPRedirectHandler, Request, build_opener

HOST = "docs.metrum.ai"
PREFIX = "/metrum-ai-bench-cli/"
START = "https://docs.metrum.ai/metrum-ai-bench-cli/latest/"


def in_scope(url):
    parsed = urlsplit(url)
    return parsed.scheme == "https" and parsed.hostname == HOST and parsed.port in (None, 443) and parsed.path.startswith(PREFIX) and parsed.username is None


class SameSiteRedirect(HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        if not in_scope(newurl):
            raise HTTPError(newurl, code, "out-of-scope redirect", headers, fp)
        return super().redirect_request(req, fp, code, msg, headers, newurl)


class Links(HTMLParser):
    def __init__(self):
        super().__init__()
        self.hrefs = []

    def handle_starttag(self, tag, attrs):
        for key, value in attrs:
            if tag.lower() in ("a", "link") and key.lower() == "href" and value:
                self.hrefs.append(value)


def crawl(start=START, limit=500):
    if not in_scope(start):
        raise ValueError("start must be the published documentation tree")
    opener = build_opener(SameSiteRedirect())
    queue = deque([urldefrag(start)[0]])
    seen = set()
    errors = []
    while queue:
        url = queue.popleft()
        if url in seen:
            continue
        if len(seen) >= limit:
            raise ValueError(f"crawl exceeded {limit} URLs")
        seen.add(url)
        try:
            with opener.open(Request(url, headers={"User-Agent": "bench-cli-docs-link-check/1"}), timeout=20) as response:
                if not in_scope(response.geturl()):
                    raise ValueError("response left documentation tree")
                content_type = response.headers.get_content_type()
                if content_type != "text/html":
                    continue
                data = response.read(5_000_001)
                if len(data) > 5_000_000:
                    raise ValueError("HTML response exceeds size limit")
                doc = Links()
                doc.feed(data.decode("utf-8", errors="replace"))
                for href in doc.hrefs:
                    target = urldefrag(urljoin(response.geturl(), href))[0]
                    if in_scope(target) and target not in seen:
                        queue.append(target)
        except (HTTPError, URLError, TimeoutError, ValueError, OSError) as exc:
            errors.append(f"{url}: {exc}")
    return seen, errors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--limit", type=int, default=500)
    args = parser.parse_args()
    if args.limit < 1:
        parser.error("limit must be positive")
    try:
        seen, errors = crawl(limit=args.limit)
    except ValueError as exc:
        print(f"documentation crawl failed: {exc}", file=sys.stderr)
        return 1
    for error in errors:
        print(error, file=sys.stderr)
    print(f"checked {len(seen)} published documentation URLs; failures={len(errors)}")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
