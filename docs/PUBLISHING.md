# Making the repository public

The repository starts private. Nothing below has been run for you. Work through it in order when you decide
to publish.

## 1. Before the visibility change

- [x] **License.** MIT, in `LICENSE`, with the GitHub handle as the copyright holder. Change the name there if
  you want your own name on it.
- [ ] **Read `CLAUDE.md` and `todo.md`.** They are notes for working on the project and mention tools and
  conventions of the machine it was built on (`freeport`, `devdb`, `devgarage`, shared dev services). Trim or
  keep them as you prefer; they contain no secrets.
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

## What the last review looked at

The security review made before publishing is summarised in the pull request or commit that added this file.
Decisions left to you are the license, the tone of `CLAUDE.md`, and the branch rules above.
