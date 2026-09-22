- **`.env.example` can no longer drift from `Settings`.** A test pins
  the 33 `SECTION__FIELD` keys the loader reads against the template,
  both directions: a setting added without documenting it — or a stale
  key left in the template — fails the build. The struct ignores
  unknown fields silently, so without this the next typo'd variable
  misbehaves exactly like the shipped-placeholder incident class.
