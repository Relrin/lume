# @lume/plugin-api

The typings of the API a Lume plugin's code is given: the global `lume` object and
`console`. A plugin's host half is written in TypeScript against them and bundled into
one JavaScript file, which Lume runs in a sandboxed QuickJS engine.

Add it as a development dependency and include it in the plugin's `tsconfig.json`:

```json
{
  "compilerOptions": { "types": ["@lume/plugin-api"] }
}
```

The typings are generated from Lume's own definitions of the API, so they say exactly
what the running application offers. The API is versioned by its major version, which a
plugin's manifest names as `api = "1"`.
