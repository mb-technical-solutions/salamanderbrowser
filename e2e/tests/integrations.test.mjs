import assert from "node:assert/strict";
import test from "node:test";
import { withApp } from "../lib/app.mjs";

async function invokeContract(app, command, args = {}) {
  try {
    return { ok: true, value: await app.invoke(command, args) };
  } catch (error) {
    return { ok: false, error: String(error) };
  }
}

async function assertCommandErrorCode(app, command, code, args = {}) {
  const error = await app.invokeError(command, args);
  assert.match(error, new RegExp(`"code":"${code}"`));
}

test("offline cloud, update, team-lock, trial, and synchronizer contracts are deterministic", async () => {
  await withApp(
    "integrations-contracts",
    async (app) => {
      await assertCommandErrorCode(
        app,
        "start_mcp_server",
        "WAYFERN_TERMS_REQUIRED",
      );
      assert.equal(await app.invoke("cloud_get_user"), null);
      assert.equal(await app.invoke("cloud_get_proxy_usage"), null);
      assert.ok(await app.invoke("cloud_get_wayfern_token"));
      assert.deepEqual(await app.invoke("get_team_locks"), []);
      assert.equal(
        await app.invoke("get_team_lock_status", {
          profileId: "00000000-0000-0000-0000-000000000000",
        }),
        null,
      );
      assert.deepEqual(await app.invoke("get_sync_sessions"), []);
      const startResult = await invokeContract(app, "start_sync_session", {
        leaderProfileId: "00000000-0000-0000-0000-000000000001",
        followerProfileIds: ["00000000-0000-0000-0000-000000000002"],
      });
      assert.equal(startResult.ok, false);
      const stopError = await app.invokeError("stop_sync_session", {
        sessionId: "missing",
      });
      assert.match(stopError, /not found|session/i);
      const removeError = await app.invokeError("remove_sync_follower", {
        sessionId: "missing",
        followerProfileId: "missing",
      });
      assert.match(removeError, /not found|session/i);

      assert.equal(await app.invoke("check_for_app_updates"), null);
      assert.equal(await app.invoke("check_for_app_updates_manual"), null);
      assert.ok(
        await invokeContract(app, "cloud_exchange_device_code", {
          code: "SALAMANDER-E2E-INVALID-CODE",
        }),
      );
      assert.ok(await invokeContract(app, "cloud_refresh_profile"));
      assert.ok(await invokeContract(app, "cloud_get_countries"));
      assert.ok(
        await invokeContract(app, "create_cloud_location_proxy", {
          name: "E2E unavailable cloud proxy",
          country: "ZZ",
          region: null,
          city: null,
          isp: null,
        }),
      );
      assert.ok(await invokeContract(app, "cloud_refresh_wayfern_token"));

      assert.ok(await invokeContract(app, "trigger_manual_version_update"));
      assert.ok(
        await invokeContract(app, "clear_all_version_cache_and_refetch"),
      );
      assert.ok(await invokeContract(app, "check_for_browser_updates"));
      await app.invoke("dismiss_update_notification", {
        notificationId: "missing-e2e-notification",
      });
      assert.deepEqual(
        await app.invoke("complete_browser_update_with_auto_update", {
          browser: "wayfern",
          newVersion: "150.0.7871.100",
        }),
        [],
      );
      const prepareError = await app.invokeError(
        "download_and_prepare_app_update",
        {
          updateInfo: {
            current_version: "0.0.0",
            new_version: "0.0.1-e2e",
            release_notes: "E2E invalid update contract",
            download_url: `${process.env.SALAMANDER_E2E_FIXTURE_URL}/invalid-update.zip`,
            is_nightly: false,
            published_at: "2026-01-01T00:00:00Z",
            manual_update_required: false,
            release_page_url: null,
            repo_update: false,
            checksums_url: null,
            asset_digest: null,
          },
        },
      );
      assert.match(prepareError, /checksum|verif|Failed to download/i);
      const versionStatus = await app.invoke("get_version_update_status");
      assert.ok(versionStatus && typeof versionStatus === "object");
      assert.equal(typeof (await app.invoke("is_default_browser")), "boolean");

      // Remote sessions and the cookie bot are brokered by the cloud backend.
      // Signed out, every one of them must fail as a code the UI can
      // translate — a raw English string from the transport would reach the
      // user untranslated, which is what the {"code":…} convention prevents.
      const notSignedIn = /"code":"CLOUD_NOT_SIGNED_IN"/;
      const missingProfileId = "00000000-0000-0000-0000-0000000000ff";
      assert.match(await app.invokeError("list_remote_sessions"), notSignedIn);
      assert.match(
        await app.invokeError("get_remote_session", {
          sessionId: "missing-e2e-session",
        }),
        notSignedIn,
      );
      assert.match(
        await app.invokeError("stop_remote_session", {
          sessionId: "missing-e2e-session",
        }),
        notSignedIn,
      );
      // The local-launch gate. Nothing has run remotely in this session, so it
      // is empty — but it must answer, because a UI that cannot read it shows
      // an enabled Run button over a profile the backend will refuse.
      const handoff = await app.invoke("get_remote_handoff_states");
      assert.ok(
        handoff && typeof handoff === "object" && !Array.isArray(handoff),
        "the handoff gate must answer with a profile-keyed object",
      );
      assert.equal(Object.keys(handoff).length, 0);

      // The transition stream is what the desktop uses instead of polling, so
      // its subscriber has to start, report itself, and stop on demand. Both
      // calls are repeated: a second start must not open a second socket, and
      // a second stop must not fail.
      assert.equal(await app.invoke("get_remote_session_events_status"), false);
      await app.invoke("start_remote_session_events");
      assert.equal(await app.invoke("get_remote_session_events_status"), true);
      await app.invoke("start_remote_session_events");
      assert.equal(await app.invoke("get_remote_session_events_status"), true);
      await app.invoke("stop_remote_session_events");
      assert.equal(await app.invoke("get_remote_session_events_status"), false);
      await app.invoke("stop_remote_session_events");
      assert.equal(await app.invoke("get_remote_session_events_status"), false);

      assert.match(
        await app.invokeError("get_cookie_bot_schedules", { scope: "mine" }),
        notSignedIn,
      );
      assert.match(
        await app.invokeError("get_cookie_bot_schedule", {
          profileId: missingProfileId,
        }),
        notSignedIn,
      );
      assert.match(
        await app.invokeError("delete_cookie_bot_schedule", {
          profileId: missingProfileId,
        }),
        notSignedIn,
      );
      // Saved site lists are cloud-backed like the schedules above, so they
      // must refuse the same way rather than appearing to work offline.
      assert.match(
        await app.invokeError("get_cookie_bot_user_templates", {}),
        notSignedIn,
      );
      assert.match(
        await app.invokeError("create_cookie_bot_user_template", {
          name: "e2e list",
          sites: ["example.com"],
        }),
        notSignedIn,
      );
      assert.match(
        await app.invokeError("update_cookie_bot_user_template", {
          id: "00000000-0000-0000-0000-000000000000",
          name: "renamed",
          sites: null,
        }),
        notSignedIn,
      );
      assert.match(
        await app.invokeError("delete_cookie_bot_user_template", {
          id: "00000000-0000-0000-0000-000000000000",
        }),
        notSignedIn,
      );
      assert.match(
        await app.invokeError("check_cookie_bot_conflicts", {
          profileId: missingProfileId,
          runAtMinute: 120,
          daysMask: 127,
        }),
        notSignedIn,
      );
      assert.match(
        await app.invokeError("get_cookie_bot_runs", { limit: 10 }),
        notSignedIn,
      );
      assert.match(
        await app.invokeError("cancel_cookie_bot_run", {
          runId: "missing-e2e-run",
        }),
        notSignedIn,
      );
      assert.match(
        await app.invokeError("get_cookie_bot_presets"),
        notSignedIn,
      );
      assert.match(
        await app.invokeError("get_remote_hours_quota"),
        notSignedIn,
      );
      assert.match(
        await app.invokeError("get_cookie_bot_usage", { period: "2026-01" }),
        notSignedIn,
      );

      // Enrolling and running act on a profile this machine holds: both are
      // refused before any network call when it does not exist, so a bad id
      // can never reach a leased host or an hour of the pooled budget.
      assert.match(
        await app.invokeError("save_cookie_bot_schedule", {
          profileId: missingProfileId,
          schedule: {
            profile_name: "E2E missing profile",
            platform: "windows",
            enabled: true,
            run_at_minute: 120,
            days_mask: 127,
            timezone: "UTC",
            preset: "balanced",
            max_minutes: 60,
            sites: ["https://example.com"],
          },
          acknowledgeConflict: false,
        }),
        /"code":"PROFILE_NOT_FOUND"/,
      );
      assert.match(
        await app.invokeError("run_cookie_bot_now", {
          profileId: missingProfileId,
          maxMinutes: 30,
        }),
        /"code":"PROFILE_NOT_FOUND"/,
      );

      const trial = await app.invoke("get_commercial_trial_status");
      assert.ok(trial && typeof trial === "object");
      await app.invoke("acknowledge_trial_expiration");
      assert.equal(await app.invoke("has_acknowledged_trial_expiration"), true);
      await app.invoke("cloud_logout");
      assert.equal(await app.invoke("cloud_get_user"), null);
    },
    { wayfernTermsAccepted: false },
  );
});
