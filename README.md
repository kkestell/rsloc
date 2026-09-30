# rsloc

Count lines of Rust source and score cognitive complexity.

The tool parses each `.rs` file and reports:

- **Prod**: lines of production code
- **Test**: lines of code inside `#[test]` functions and `#[cfg(test)]`
  modules
- **Doc**: comment-only lines outside test code
- **Cog**: cognitive complexity of the production code, following G. Ann
  Campbell's specification

Blank lines are not counted.

## Install

```sh
make install    # builds a release binary into ~/.local/bin
```

## Usage

```
rsloc [OPTIONS] [FILES OR DIRECTORIES]...
```

rsloc searches the given paths for `.rs` files, recursing into
directories. The default path is the current directory.

- `--exclude-dir <names>`: comma-separated directory names to skip.
  Defaults to `.git,.hg,.svn,target`. Passing this option replaces the
  defaults, so include them again if you want them.
- `-n`, `--exclude-file <names>`: comma-separated file names to skip.
- `-f`, `--format <format>`: `tabular` (default) or `json`.
- `-i`, `--items`: list each file's top-level items instead of one row
  per file. Methods and module contents are nested beneath their items.
- `-h`, `--help`: print help.

Excluded names must match the whole file or directory name. Globs and
paths are not supported.

## Examples

```
$ rsloc
File         Prod  Test  Doc  Cog
─────────────────────────────────
src/main.rs   922   225   21  145
─────────────────────────────────
              922   225   21
```

```
$ rsloc -i src
Location          Item                         Kind    Prod  Doc  Cog
─────────────────────────────────────────────────────────────────────
src/main.rs:19    Counts                       struct     7    0    0
src/main.rs:28    Cli                          struct    19    1    0
src/main.rs:63    impl From<&Cli> for Options  impl      16    0    0
src/main.rs:64      from                       fn        14    0    0
...
```

In the footer, complexity is left blank because a sum of complexity
across files is not meaningful. With `--items`, the footer shows the
file totals, so it also counts lines that belong to no item, such as
`use` declarations.

With `-f json`, the output is an array with one object per file. When
`--items` is also set, each object has a nested `items` array.
