# Making the repository public

The repository starts private. Nothing below has been run for you. Work through it in order when you decide
to publish.

## 1. Before the visibility change

- [x] **License.** MIT, in `LICENSE`, with the GitHub handle as the copyright holder. Change the name there if
  you want your own name on it.
- [x] **`CLAUDE.md` and `todo.md`.** They were trimmed of notes about the machine the project was built on.
  `todo.md` is a public roadmap with known limits. Read them once more and keep what you want shown.
- [ ] **Decide who may push to `main`.** The release job pushes the version bump to `main` with the workflow
  token. A rule that requires pull requests for `main` must let the GitHub Actions app bypass it, or the bump
  fails. Make a pull request the normal way in, and keep direct pushes for the release job only.
- [ ] **Scan the history once more** for secrets, for example with `gitleaks detect` or `trufflehog git file://.`.
  The history was checked for the values in the development `.env` and nothing was found. The `.env` is
  ignored by git and by the Docker build context.
- [ ] **Rotate the development credentials** if the development database or bucket is reachable from outside
  your network. They were never committed, this is only a precaution.

## 2. Make it public

```bash
gh repo edit beyto1974/bulut --visibility public --accept-visibility-change-consequences
```

## 3. Turn on the GitHub security features

Free for public repositories. Run each command, or use **Settings -> Advanced Security** for the same.

```bash
REPO=beyto1974/bulut

# Secret scanning, and push protection that blocks a push containing a secret.
gh api -X PATCH repos/$REPO \
  -f 'security_and_analysis[secret_scanning][status]=enabled' \
  -f 'security_and_analysis[secret_scanning_push_protection][status]=enabled'

# Dependabot alerts and automatic security update pull requests (version updates come from
# .github/dependabot.yml).
gh api -X PUT repos/$REPO/vulnerability-alerts
gh api -X PUT repos/$REPO/automated-security-fixes

# Private vulnerability reporting, which SECURITY.md points people to.
gh api -X PUT repos/$REPO/private-vulnerability-reporting

# Code scanning with CodeQL, default setup.
gh api -X PATCH repos/$REPO/code-scanning/default-setup \
  -f state=configured -f query_suite=default \
  -f 'languages[]=actions' -f 'languages[]=javascript-typescript' -f 'languages[]=rust'

# Workflows get a read-only token by default and cannot approve pull requests.
gh api -X PUT repos/$REPO/actions/permissions/workflow \
  -f default_workflow_permissions=read -F can_approve_pull_request_reviews=false
```

Then in **Settings -> Actions -> General**, set *Fork pull request workflows from outside collaborators* to
**Require approval for all outside collaborators**. Protect `main` (**Settings -> Rules**): require a pull
request and the `Format, lint & scripts`, `Tests` and `End-to-end` checks, block force pushes and deletion,
and let the GitHub Actions app bypass it for the release commit.

## 4. The container image

The first push to `main` publishes `ghcr.io/beyto1974/bulut`. A package is private by default, even when the
repository is public. Open the package page (**Packages** on the repository, then **Package settings**) and
change the visibility to **Public** so `docker pull` works without a login. It is already linked to the
repository through the `org.opencontainers.image.source` label.

## 5. Tell people what they need to know

The README opens with the fact that Bulut has no authentication and must run behind basic auth and a bearer
token. Keep that warning where it is.

## Security review before publication

A review of the whole tree was made before the first push (code, workflows, Dockerfile, examples and the
contents of the repository). Findings and what was done:

| Finding | Severity | Status |
| --- | --- | --- |
| `/mcp` accepted any content type and any `Origin`, so a web page could make a browser with cached basic-auth credentials write through it | medium | Fixed: JSON only, other origins refused |
| One session could be filled without limit through versions, bytes, unfinished uploads or tags | medium | Fixed: `MAX_VERSIONS_PER_FILE`, `MAX_SESSION_BYTES`, `MAX_PENDING_UPLOADS`, `MAX_TAGS_PER_VERSION`, enforced in the database under a lock |
| Notes and descriptions could fake headings and links in `llms.txt` with `\r` or `U+2028` | low | Fixed, and the text says user text is data |
| Script injection pattern and unlocked npm install in CI | low | Fixed: values go through `env`, the version is validated, the e2e dependency has a lockfile |
| Dependencies with advisories (old `hyper`, `h2`, `rustls`, `rustls-webpki` from the AWS SDK's legacy TLS feature) | medium | Fixed: modern HTTPS client, `cargo audit` is clean. One advisory (`rsa`) is ignored on purpose, it is only in the lockfile through an optional, unused MySQL driver (see `.cargo/audit.toml`) |
| Content type accepted any ASCII, including control characters | info | Fixed: printable characters only |
| Notes about the build machine in `CLAUDE.md` and `todo.md` | info | Fixed |
| Memory held per in-flight part before the upload id is checked, no read timeouts | low | Not changed: connection limits and timeouts belong to the reverse proxy, the README says so |
| Unlimited number of sessions, and any request keeps a session alive | design | Not changed: rate limits belong to the proxy. A session code is an identifier, not a secret |
| Objects can stay in the bucket if a session is purged while an upload into it completes | low | Not changed, listed in `todo.md` |

Checked and fine: every SQL statement is parameterised, every lookup by id is scoped to the session, object
keys are made only of the validated code and a random id, downloads are attachments with a sandboxing CSP,
the UI builds the page with DOM calls only under a strict CSP, secrets are not logged, the image holds only the
binary and runs as non-root, and the workflows use no `pull_request_target`, minimal permissions and pinned
actions.

Decisions left to you: the branch rules above, and whether the copyright line in `LICENSE` should carry your
name instead of the GitHub handle.
