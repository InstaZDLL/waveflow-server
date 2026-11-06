#!/usr/bin/env python3
"""Creates the GitLab release of a tag, its files linked rather than attached.

    python3 scripts/release/gitlab-release.py <tag> <notes.md> <assets-dir> <download-base>

Every file in <assets-dir> becomes a link to <download-base>/<name>: the R2 copy
on download.waveflow.app, since a project's storage quota would not take five
archives per version. The links are the files' names and nothing else, so a
release page cannot name a file the mirror does not hold.

Run in two places, with two ways in:

- by .gitlab-ci.yml, where GitLab is the home: CI_API_V4_URL, CI_PROJECT_ID and
  CI_JOB_TOKEN come with the job;
- by release.yml, which keeps the GitLab backup's releases current while GitHub
  is the home: GITLAB_PROJECT (e.g. waveflow-groupe/waveflow-server) and
  GITLAB_API_TOKEN, a token with the `api` scope on that project. A project
  access token needs a paid tier on GitLab.com, so on the Free tier this is
  left unset and the step skipped; the releases are rebuilt from the tags at
  the switch (docs/guides/releasing.md). GITLAB_API_URL defaults to
  https://gitlab.com/api/v4, and GITLAB_TAG_WAIT (600) is how many seconds
  to wait for the tag to arrive;
- by hand, with the same two variables, to recreate a release the backup
  lacks.

A release that already exists keeps its description, as on GitHub: it was
written once, and a retry is not meant to rewrite it. Its missing links are
added. The tag itself must already be on GitLab; this never creates one, since
a tag GitLab made would be neither the signed tag nor necessarily its commit.
"""

import json
import os
import pathlib
import sys
import time
import urllib.error
import urllib.parse
import urllib.request


def api():
    """Returns a function calling the project's REST API with whichever token
    the environment provides: GITLAB_API_TOKEN as a bearer, or the CI job's
    own token."""
    base = os.environ.get("CI_API_V4_URL") or os.environ.get("GITLAB_API_URL") or "https://gitlab.com/api/v4"
    token = os.environ.get("GITLAB_API_TOKEN")
    if token:
        project = os.environ.get("GITLAB_PROJECT") or os.environ.get("CI_PROJECT_ID")
        # Bearer rather than PRIVATE-TOKEN: GitLab takes a personal or project
        # token that way too, and an OAuth token (glab's) only that way.
        header = ("Authorization", f"Bearer {token}")
    else:
        project = os.environ.get("CI_PROJECT_ID")
        header = ("JOB-TOKEN", os.environ.get("CI_JOB_TOKEN", ""))
    if not project or not header[1]:
        sys.exit("set GITLAB_API_TOKEN and GITLAB_PROJECT, or run inside a GitLab CI job")
    root = f"{base.rstrip('/')}/projects/{urllib.parse.quote(project, safe='')}"

    def call(method, path, body=None):
        """Sends one request under the project and returns its status and
        decoded JSON body; an HTTP error is raised as urllib's HTTPError."""
        data = None if body is None else json.dumps(body).encode()
        request = urllib.request.Request(root + path, data=data, method=method)
        request.add_header(*header)
        request.add_header("Content-Type", "application/json")
        with urllib.request.urlopen(request, timeout=60) as response:
            return response.status, json.loads(response.read() or b"null")

    return call


def main():
    """Creates the release of the tag named on the command line, or adds the
    links an existing one lacks, leaving its description as it is."""
    if len(sys.argv) != 5:
        sys.exit(__doc__.strip().splitlines()[2].strip())
    tag, notes, assets, base = sys.argv[1:]
    base = base.rstrip("/")
    call = api()
    quoted = urllib.parse.quote(tag, safe="")

    # A tag pipeline has its tag by definition, and a job token may not read
    # the repository API; only the mirror from GitHub has to ask. It asks for a
    # while: the tag reaches GitLab through mirror-gitlab.yml, a separate run
    # that may still be queued when the release gets here, and giving up at the
    # first 404 would leave the release without its page for good.
    if os.environ.get("GITLAB_API_TOKEN"):
        deadline = time.monotonic() + int(os.environ.get("GITLAB_TAG_WAIT", "600"))
        while True:
            try:
                call("GET", f"/repository/tags/{quoted}")
                break
            except urllib.error.HTTPError as err:
                if err.code != 404:
                    raise
            if time.monotonic() >= deadline:
                sys.exit(f"{tag} is still not on GitLab: push the tag first (mirror-gitlab.yml does)")
            print(f"{tag} is not on GitLab yet; asking again in 30 s", flush=True)
            time.sleep(30)

    links = [
        {
            "name": path.name,
            "url": f"{base}/{path.name}",
            "link_type": "package" if path.name.startswith("waveflow-server-") else "other",
        }
        for path in sorted(pathlib.Path(assets).iterdir())
        if path.is_file()
    ]
    if not links:
        sys.exit(f"{assets} holds no file to link")

    description = pathlib.Path(notes).read_text(encoding="utf-8")
    if "-" in tag.removeprefix("v"):
        description = "**Pre-release.**\n\n" + description

    try:
        call("POST", "/releases", {"tag_name": tag, "name": tag, "description": description, "assets": {"links": links}})
        print(f"GitLab release {tag} created with {len(links)} links")
        return
    except urllib.error.HTTPError as err:
        if err.code != 409:
            print(err.read().decode(errors="replace"), file=sys.stderr)
            raise

    _, existing = call("GET", f"/releases/{quoted}/assets/links")
    have = {link["name"] for link in existing}
    added = 0
    for link in links:
        if link["name"] not in have:
            call("POST", f"/releases/{quoted}/assets/links", link)
            added += 1
    print(f"GitLab release {tag} already exists; {added} missing links added")


if __name__ == "__main__":
    main()
