# SDK Registry Account Setup — Step by Step

## PyPI (Python)

1. Go to https://pypi.org/account/register/
2. Create account with your email
3. Verify email
4. Go to https://pypi.org/manage/account/token/
5. Click "Add API token" → scope "Entire account" → copy token (starts with `pypi-`)
6. In GitHub repo: Settings → Secrets → New → name: `PYPI_TOKEN`, value: the token
7. Test: `cd sdks/python && pip install build twine && python -m build && twine upload --repository testpypi dist/*`

## npm (TypeScript)

1. Go to https://www.npmjs.com/signup
2. Create account
3. Go to https://www.npmjs.com/settings/YOUR_USERNAME/tokens
4. Click "Generate New Token" → Classic → Publish
5. Copy token
6. GitHub secret: `NPM_TOKEN`
7. Test: `cd sdks/typescript && npm install && npx tsc && npm publish --dry-run`

## crates.io (Rust)

1. Go to https://crates.io/ → click "Log in with GitHub"
2. Go to https://crates.io/settings/tokens → "New Token" → publish scope
3. Copy token
4. GitHub secret: `CRATES_IO_TOKEN`
5. Test: `cd sdks/rust && cargo package --list`

## Maven Central (Java)

1. Go to https://central.sonatype.com/ → Sign up
2. You need to verify domain ownership for `dev.sutradb` group
   - Easiest: add a TXT DNS record, OR
   - Use `io.github.emmaleonhart` as group (auto-verified for GitHub users)
3. Go to Account → Generate User Token
4. GitHub secrets: `MAVEN_USERNAME` (token user), `MAVEN_PASSWORD` (token pass)
5. Note: Maven Central requires GPG-signed artifacts. Run `gpg --gen-key` locally first.

## NuGet (.NET)

1. Go to https://www.nuget.org/ → Sign in with Microsoft account
2. Go to https://www.nuget.org/account/apikeys
3. Click "Create" → name it, set package scope
4. Copy API key
5. GitHub secret: `NUGET_TOKEN`
6. Test: `cd sdks/dotnet && dotnet pack -c Release`

## Go Modules

No account needed. Just tag and push:
```bash
git tag sdks/go/v0.1.0
git push --tags
```
Go module proxy picks it up automatically within minutes.

## After All Accounts Are Set Up

```bash
# Update all versions
# Then tag and push to trigger CI publish:
git tag v0.1.0
git push --tags
```
