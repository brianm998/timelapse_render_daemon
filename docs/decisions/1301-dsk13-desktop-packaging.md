# DSK-13: desktop packaging, self-test and release decisions

- Release ProGuard is off. Its default rules do not cover the reflection in grpc, protobuf-lite and
  jmdns, and a broken release build would only show in an installed package.
- The packaged runtime's modules are what `jdeps --print-module-deps` reports for the runtime
  classpath (`java.base` and `java.desktop` are Compose's defaults), plus `java.logging` and
  `jdk.crypto.ec`. The latter is needed by pinned TLS, which the self-test's plaintext `--server`
  call does not exercise. Compose's `suggestModules` needs an artifact that was not in the offline
  Gradle cache.
- `desktop-packages` runs on `needs.version.outputs.build == 'true'`, not `inputs.build != false`:
  push and schedule runs have no inputs and `null == false` is true. Spec 2 §8 now says so.
- `kotlin-mdns` passes `--no-build-cache`, because Gradle does not track `CI` and `TRM_MDNS_TEST`
  and could restore a cached result instead of running the test.
- `SnapshotRepository` takes an optional initial snapshot, so the self-test shows its built-in sample
  through `ActivityPresenter` without a server.
- The self-test's screen classes are a list; `SelfTestTest` fails when a file under `ui` is not on it.
- The macOS jobs need only the Developer ID Application identity: a `.dmg` has no installer signature.
- The `kotlin-test` floor (361) is 90 % of 402 passing cases counted from the `@Test` annotations
  (420, less the 18 contract tests and the one mDNS test that are skipped, plus
  `check_system_tests_skipped`), not read from a run.
