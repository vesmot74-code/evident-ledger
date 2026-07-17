# Database Migration Policy

Current database contents are development/test fixtures only.

They are not production data and do not require data migration.

Implementation migrates SCHEMA only:

- new tables
- new indexes
- new constraints

Data migration is not required.

Before v1 rollout:

- discard development/test database
- recreate clean database
- apply migrations from zero (`sqlx migrate run`)
- run API contract tests against clean database
