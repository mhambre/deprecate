# Catalog format

`deprecations.json` is a versioned, machine-readable summary of a crate's declared
deprecations. Generate it before publishing:

```console
cargo deprecate catalog
```

Schema version 1 contains package identity followed by deprecation records:

```json
{
  "schema": 1,
  "crate": "example",
  "version": "2.8.0",
  "deprecations": [
    {
      "kind": "item",
      "name": "new_client",
      "since": "2.1.0",
      "remove": "3.0.0",
      "replacement": "Client::builder",
      "reason": "client construction is now configurable",
      "migrate": "new_client($url) => Client::builder().url($url).build()"
    }
  ]
}
```

`kind` is `item` or `feature`. `name` and `since` are required. `remove`,
`replacement`, `reason`, and `migrate` are optional. Readers must ignore unknown
keys within a known schema version so compatible fields can be added later.

Dependency catalogs must match the resolved Cargo package's exact `crate` name
and `version`, including prerelease and build metadata. Dependency aliases do not
change that identity. Mismatches are errors; regenerate the catalog for the
published version. Lifecycle shorthand such as `2` is not a catalog package version.

Commit the catalog so release diffs can compare it with older revisions. Include
it in the published crate package when using an explicit Cargo `include` list.
