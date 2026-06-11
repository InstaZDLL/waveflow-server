// Conventional commits + project-wide tightening. Enforced via the
// husky `commit-msg` hook installed by `bun install`'s `prepare:
// husky` step. Mirrors the desktop's `.commitlintrc.cjs` so a
// contributor who reads either repo gets the same contract.
module.exports = {
  extends: ['@commitlint/config-conventional'],
  rules: {
    // Header keeps PR titles readable in dense GitHub listings and
    // fits unbroken in a 100-col terminal git log.
    'header-max-length': [2, 'always', 100],
    // Subject in lowercase — `subject-case: ['lower-case']` is the
    // anti-pascal/sentence/start/upper-case rule. Conventional
    // commits' default would accept any of those.
    'subject-case': [2, 'always', 'lower-case'],
    // Scopes use kebab-case so a future grep for, say,
    // `feat(home-banner):` lands every commit consistently.
    'scope-case': [2, 'always', 'kebab-case'],
  },
}
