# SDK Publishing Setup Guide

Instructions for configuring each SDK's package registry secrets so the `publish-sdks.yml` CI workflow can publish automatically on version tags.

## How it works

1. Push a version tag: `git tag v0.1.0 && git push --tags`
2. CI runs `publish-sdks.yml` which publishes all SDKs
3. Each registry needs a token stored as a GitHub secret

## Setting GitHub Secrets

Go to: **GitHub repo → Settings → Secrets and variables → Actions → New repository secret**

---

## Python → PyPI

**Secret name:** `PYPI_TOKEN`

1. Go to https://pypi.org/manage/account/token/
2. Create a new API token (scope: entire account, or project-scoped after first publish)
3. Copy the token (starts with `pypi-`)
4. Add as GitHub secret `PYPI_TOKEN`

**First publish (manual):**
```bash
cd sdks/python
pip install build twine
python -m build
twine upload dist/*
# Enter username: __token__
# Enter password: pypi-YOUR-TOKEN
```

---

## TypeScript → npm

**Secret name:** `NPM_TOKEN`

1. Go to https://www.npmjs.com/settings/YOUR_USERNAME/tokens
2. Click "Generate New Token" → "Classic Token" → "Publish"
3. Copy the token
4. Add as GitHub secret `NPM_TOKEN`

**First publish (manual):**
```bash
cd sdks/typescript
npm install
npx tsc
npm login
npm publish --access public
```

---

## Rust → crates.io

**Secret name:** `CRATES_IO_TOKEN`

1. Go to https://crates.io/settings/tokens
2. Click "New Token" with publish scope
3. Copy the token
4. Add as GitHub secret `CRATES_IO_TOKEN`

**First publish (manual):**
```bash
cd sdks/rust
cargo login YOUR_TOKEN
cargo publish
```

**Note:** The `sdks/rust/Cargo.toml` must have unique `name`, `version`, `description`, `license`, `repository` fields. crates.io requires `license` and `description`.

---

## Java → Maven Central (OSSRH)

**Secret names:** `MAVEN_USERNAME`, `MAVEN_PASSWORD`

This is the most complex setup. Maven Central uses Sonatype OSSRH.

1. Create account at https://central.sonatype.com/
2. Claim the `dev.sutradb` group ID (requires domain verification or GitHub proof)
3. Generate a user token at https://central.sonatype.com/account
4. Add `MAVEN_USERNAME` (token username) and `MAVEN_PASSWORD` (token password) as GitHub secrets

**Additional setup needed in `sdks/java/pom.xml`:**
- Add `<distributionManagement>` section pointing to OSSRH
- Add `maven-gpg-plugin` for signing (Central requires signed artifacts)
- Add `nexus-staging-maven-plugin` for automated release

**First publish is usually manual and requires GPG signing.**

---

## .NET → NuGet

**Secret name:** `NUGET_TOKEN`

1. Go to https://www.nuget.org/account/apikeys
2. Click "Create" → name it, set expiration, scope to your package
3. Copy the API key
4. Add as GitHub secret `NUGET_TOKEN`

**First publish (manual):**
```bash
cd sdks/dotnet
dotnet pack -c Release
dotnet nuget push bin/Release/*.nupkg --api-key YOUR_KEY --source https://api.nuget.org/v3/index.json
```

---

## Go → Go Module Proxy

**No secrets needed!** Go modules are published automatically.

1. Tag the Go module: `git tag sdks/go/v0.1.0`
2. Push the tag: `git push --tags`
3. The Go module proxy (proxy.golang.org) picks it up automatically

**First time:** Users can immediately `go get github.com/EmmaLeonhart/SutraDB/sdks/go@v0.1.0`

---

## Version Tagging

When ready to release:

```bash
# Update version numbers in all SDK configs first:
# - sdks/python/pyproject.toml → version
# - sdks/typescript/package.json → version
# - sdks/rust/Cargo.toml → version
# - sdks/java/pom.xml → version
# - sdks/dotnet/SutraDB.Client.csproj → Version
# - sdks/go/go.mod (module path stays the same)

# Then tag and push
git tag v0.1.0
git push --tags

# For Go specifically
git tag sdks/go/v0.1.0
git push --tags
```

## Checklist Before First Publish

- [ ] Create PyPI account and token → `PYPI_TOKEN`
- [ ] Create npm account and token → `NPM_TOKEN`
- [ ] Create crates.io account and token → `CRATES_IO_TOKEN`
- [ ] Create Sonatype account, claim group → `MAVEN_USERNAME`, `MAVEN_PASSWORD`
- [ ] Create NuGet account and API key → `NUGET_TOKEN`
- [ ] Update version numbers in all SDKs
- [ ] Test manual publish for each SDK first
- [ ] Then push tag to trigger CI publish
