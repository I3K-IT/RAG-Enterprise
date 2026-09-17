# changelog.d — one file per change

Put your CHANGELOG entry here, in its own file, instead of editing
`CHANGELOG.md` directly. `scripts/release-changelog.sh` collects these
files into a version section when a release is cut, and deletes them.

## Why

Every pull request used to add its bullet at the same anchor — the top
of the section under `## [Unreleased]`. Two branches cut from the same
`main` therefore collided there by construction, whoever wrote them.
That is not a contributor mistake; it is the shape of the file.

It was also not harmless. The line `fix/enricher-length-contract` sat in
`CHANGELOG.md` on `main` for a while: the tail of a `>>>>>>> ` conflict
marker that survived a hand resolution.

Two files with different names cannot conflict. That is the whole idea.

## How

Name the file `<section>-<short-slug>.md`, where `<section>` is one of:

    added  changed  performance  deprecated  removed  fixed  security

The slug is yours — the branch name works well. Examples:

    changelog.d/fixed-eullm-ctx-validation.md
    changelog.d/added-webhook-notifications.md

The file holds the bullet exactly as it should read in `CHANGELOG.md`,
with no section heading of its own:

```markdown
- **One line saying what changed, in bold.** Then the explanation: what
  was wrong before, what the reader would have seen, and what happens
  now. Wrapped at the same width as the rest of the file.
```

The two files sitting here now are real entries waiting for the next
release — read them as the worked example.

## Releasing

    scripts/release-changelog.sh 0.1.45

That moves every fragment into a new `## [0.1.45] - <today>` section in
`CHANGELOG.md`, grouped by section in the order above, and removes the
fragment files. Check the result before committing: the script edits,
it does not commit.
