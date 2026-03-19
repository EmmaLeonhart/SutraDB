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

## Java → Maven Central (Central Portal)

**Build system:** Gradle (Kotlin DSL)
**Secret names:** `MAVEN_USERNAME`, `MAVEN_TOKEN`, `GPG_PRIVATE_KEY`, `GPG_PASSPHRASE`

Maven Central is the most complex setup — it requires GPG-signed artifacts and a Sonatype Central Portal account.

### 1. Sonatype Central Portal Account

1. Create account at https://central.sonatype.com/
2. Claim the `io.github.emmaleonhart` namespace (auto-verified for GitHub users)
3. Go to **Account → Generate User Token**
4. You'll get a **token username** and **token password** — these are NOT your login credentials
5. Add as GitHub secrets:
   - `MAVEN_USERNAME` → token username
   - `MAVEN_TOKEN` → token password

### 2. GPG Key for Artifact Signing

Maven Central requires ALL artifacts (.jar, .pom, -sources.jar, -javadoc.jar) to be GPG-signed.

**Generate a GPG key:**
```bash
gpg --full-generate-key
# Choose: RSA and RSA, 4096 bits, does not expire
# Enter your name and email (use your GitHub email)
# Set a passphrase (you'll need this as GPG_PASSPHRASE secret)
```

**Publish the public key to a keyserver** (Maven Central verifies signatures):
```bash
# List your keys to find the key ID
gpg --list-keys --keyid-format long

# Upload to Ubuntu keyserver (Maven Central checks this)
gpg --keyserver keyserver.ubuntu.com --send-keys YOUR_KEY_ID

# Also upload to keys.openpgp.org as backup
gpg --keyserver keys.openpgp.org --send-keys YOUR_KEY_ID
```

**Export the private key for CI:**
```bash
# Export as ASCII-armored for safe storage in GitHub secrets
gpg --export-secret-keys --armor YOUR_KEY_ID
# Copy the output (including BEGIN/END lines) as the GPG_PRIVATE_KEY secret
```

**Add as GitHub secrets:**
- `GPG_PRIVATE_KEY` → the ASCII-armored private key (Gradle's in-memory signing reads this directly)
- `GPG_PASSPHRASE` → the passphrase you set during key generation

### 3. How CI Publishing Works

The `publish-sdks.yml` workflow:
1. `gradle/actions/setup-gradle` caches Gradle dependencies
2. GPG signing uses Gradle's in-memory PGP keys (from `GPG_PRIVATE_KEY` env var — no GPG binary needed)
3. `./gradlew publishMavenJavaPublicationToCentralRepository` builds, signs, and uploads
4. Credentials are read from `MAVEN_USERNAME` / `MAVEN_TOKEN` env vars

### 4. Building Locally

```bash
cd sdks/java

# Build + test (no GPG needed)
./gradlew build

# Publish (requires GPG key + Sonatype credentials as env vars)
GPG_PRIVATE_KEY="$(gpg --export-secret-keys --armor YOUR_KEY_ID)" \
GPG_PASSPHRASE="your-passphrase" \
MAVEN_USERNAME="token-username" \
MAVEN_TOKEN="token-password" \
./gradlew publishMavenJavaPublicationToCentralRepository
```

### 5. First Publish Checklist

- [ ] Sonatype Central Portal account created
- [ ] `dev.sutradb` namespace claimed and verified
- [ ] GPG key generated and public key uploaded to keyserver
- [ ] GitHub secrets set: `MAVEN_USERNAME`, `MAVEN_TOKEN`, `GPG_PRIVATE_KEY`, `GPG_PASSPHRASE`
- [ ] Test locally: `./gradlew build` (compile + tests, no credentials needed)

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
# - sdks/java/build.gradle.kts → version
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
- [ ] Create Sonatype account, claim group → `MAVEN_USERNAME`, `MAVEN_TOKEN`
- [ ] Generate GPG key, upload to keyserver → `GPG_PRIVATE_KEY`, `GPG_PASSPHRASE`
- [ ] Create NuGet account and API key → `NUGET_TOKEN`
- [ ] Update version numbers in all SDKs
- [ ] Test manual publish for each SDK first
- [ ] Then push tag to trigger CI publish
