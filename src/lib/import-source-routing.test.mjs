import assert from "node:assert/strict";
import test from "node:test";
import {
  basenameOf,
  mergeArchiveBatchItems,
  routeImportSources,
} from "./import-source-routing.ts";

/**
 * This pins the routing table the unified importer depends on: archives are
 * the only kind that batches, a folder or data directory only makes sense
 * alone, and anything else surfaces a clear error instead of silently
 * picking one of the dropped paths.
 */

function classified(path, info) {
  return { path, info };
}

test("a single archive routes to a batch of one", () => {
  const route = routeImportSources([
    classified("a.salamander-profile", { kind: "archive" }),
  ]);
  assert.deepEqual(route, {
    type: "archive_batch",
    paths: [classified("a.salamander-profile", { kind: "archive" })],
  });
});

test("multiple archives all batch together", () => {
  const paths = [
    classified("a.salamander-profile", { kind: "archive" }),
    classified("b.salamander-profile", {
      kind: "archive",
      needs_password: true,
    }),
  ];
  assert.deepEqual(routeImportSources(paths), {
    type: "archive_batch",
    paths,
  });
});

test("a single profile folder routes directly", () => {
  const route = routeImportSources([
    classified("C:/profiles/old", { kind: "profile_folder" }),
  ]);
  assert.deepEqual(route, {
    type: "profile_folder",
    path: "C:/profiles/old",
  });
});

test("a single data directory routes directly", () => {
  const route = routeImportSources([
    classified("C:/DonutBrowser", { kind: "data_directory" }),
  ]);
  assert.deepEqual(route, {
    type: "data_directory",
    path: "C:/DonutBrowser",
  });
});

test("more than one profile folder is a mixed error, not a silent pick", () => {
  const route = routeImportSources([
    classified("a", { kind: "profile_folder" }),
    classified("b", { kind: "profile_folder" }),
  ]);
  assert.equal(route.type, "mixed_error");
});

test("an archive dropped alongside a folder is a mixed error", () => {
  const route = routeImportSources([
    classified("a.salamander-profile", { kind: "archive" }),
    classified("b", { kind: "profile_folder" }),
  ]);
  assert.equal(route.type, "mixed_error");
});

test("an unrecognized single path is an unknown error, not a crash", () => {
  const route = routeImportSources([
    classified("notes.txt", { kind: "unknown" }),
  ]);
  assert.equal(route.type, "unknown_error");
});

test("no paths at all is an unknown error", () => {
  assert.equal(routeImportSources([]).type, "unknown_error");
});

test("basenameOf handles both path separators and trailing slashes", () => {
  assert.equal(basenameOf("C:\\Users\\a\\data"), "data");
  assert.equal(basenameOf("/home/a/data/"), "data");
  assert.equal(basenameOf("single"), "single");
});

test("merging archive batch items skips paths already in the batch", () => {
  const existing = [
    {
      path: "a.salamander-profile",
      displayName: "a.salamander-profile",
      needsPassword: false,
      password: "",
      status: "success",
      resultNames: ["Already imported"],
    },
  ];
  const merged = mergeArchiveBatchItems(existing, [
    classified("a.salamander-profile", { kind: "archive" }),
    classified("b.salamander-profile", {
      kind: "archive",
      needs_password: true,
      preview_names: ["Shop", "Shop (backup)"],
    }),
  ]);

  assert.equal(merged.length, 2);
  // The existing, already-imported row is untouched — re-dropping the same
  // file must not reset its progress.
  assert.equal(merged[0].status, "success");
  assert.deepEqual(merged[0].resultNames, ["Already imported"]);
  // The new file is appended as a fresh pending row with its preview data —
  // an archive is a bundle of 1..N profiles, so previews are a list.
  assert.deepEqual(merged[1], {
    path: "b.salamander-profile",
    displayName: "b.salamander-profile",
    previewNames: ["Shop", "Shop (backup)"],
    needsPassword: true,
    password: "",
    status: "pending",
  });
});

test("merging ignores non-archive classifications (defensive: callers only merge after routing to archive_batch)", () => {
  const merged = mergeArchiveBatchItems(
    [],
    [classified("folder", { kind: "profile_folder" })],
  );
  assert.deepEqual(merged, []);
});
