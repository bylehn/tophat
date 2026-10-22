---
name: th-data
description: Manage data roots and datasets: configure where data lives, register raw data, declare data of unknown history, publish a study output as a dataset, verify, retire, and trace lineage. Use when a study needs data that is not registered yet, or when the human asks about data. Not for moving or copying data; TopHat never does that.
---

<!-- TopHat-managed: canonical skill text. `copier update` changes it; extend it in
.tophat/user/skills/th-data/local.md. -->

# th-data

## Data roots

A data root is a named place where data lives, such as `raw` or `results`. Names are declared in
`.tophat/roots.yaml` (committed); each machine says where a root is in `.tophat/roots.local.yaml`
(not committed):

```yaml
roots:
  raw: /absolute/path/on/this/machine
```

Stage outputs go to the `results` root, which defaults to the git-ignored `results/` folder of the
project; configure it only to put outputs elsewhere. Add a root name to `.tophat/roots.yaml` when
the human needs one. Write a location to
`.tophat/roots.local.yaml` only as the human gives it, as an absolute path. Show the current roots
with `tophat data roots`.

## Registering data

- **Raw data** with a known source: `tophat data add <name> --root <root> --path <path> --source
  <text>`, with `--date` and `--license` when known. Ask the human for the source, date and licence;
  never invent them.
- **Data of unknown history** (processed files whose origin is unclear): `tophat data declare <name>
  --root <root> --path <path> --statement <text>`, with the human's own statement of what is known.
  Declared data is labelled as such wherever it is used.
- Add `--visibility private` for data that must not reach public studies.

`tophat` hashes every file and writes the manifest under `datasets/`; never write or edit a
manifest yourself. Stages reference a dataset as `dataset:<name>@v<n>`.

## Publishing, verifying, retiring

- Publish a sealed, clean stage output for other studies: `tophat data publish <study> --stage
  <stage> --variant <variant> --as <name>`.
- Verify that registered files are unchanged: `tophat data verify [<name>[@v<n>]]`.
- Retire a version only for a defect the human confirms, with their reason: `tophat data retire
  <name>@v<n> --reason <text>`.

## Lineage

Show what a dataset or output derives from with `tophat data lineage <name>@v<n>` (or
`<study>/<stage>@<key>`), and who uses a dataset with `tophat data used-by <name>`.

Never let private data reach a public study, and never copy, move or delete data.

## Project Extensions

Read `.tophat/user/skills/th-data/local.md` and follow its instructions. Where they conflict with
this skill, `local.md` wins.
