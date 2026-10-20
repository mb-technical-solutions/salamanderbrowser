"use client";

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";
import { LuLoaderCircle } from "react-icons/lu";
import { LoginScreen, type LoginScreenState } from "@/components/login-screen";
import type { AuthStatusDto } from "@/types";

type GateState =
  | { kind: "checking" }
  | { kind: "authenticated" }
  | LoginScreenState;

function messageFromError(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function fromStatus(status: AuthStatusDto): GateState {
  switch (status.status) {
    case "authenticated":
      return { kind: "authenticated" };
    case "loginRequired":
      return { kind: "idle" };
    case "error":
      return { kind: "error", message: status.message };
  }
}

/**
 * Gates the whole app shell behind Authentik sign-in. Mirrors
 * `I18nProvider`'s shape: an async check runs once, and `children` is only
 * ever rendered once it resolves to authenticated — everything below this
 * (including `page.tsx`'s profile-loading hooks) simply never mounts while
 * signed out.
 */
export function AuthGateProvider({ children }: { children: React.ReactNode }) {
  const [state, setState] = useState<GateState>({ kind: "checking" });

  const bootstrap = useCallback(() => {
    setState({ kind: "checking" });
    invoke<AuthStatusDto>("auth_bootstrap")
      .then((status) => setState(fromStatus(status)))
      .catch((error) =>
        setState({ kind: "error", message: messageFromError(error) }),
      );
  }, []);

  useEffect(() => {
    bootstrap();

    const unlistenPromise = listen<AuthStatusDto>(
      "auth-state-changed",
      (event) => setState(fromStatus(event.payload)),
    );

    return () => {
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, [bootstrap]);

  const handleSignIn = useCallback(() => {
    setState({ kind: "waitingForBrowser" });
    invoke("auth_start_login").catch((error) =>
      setState({ kind: "error", message: messageFromError(error) }),
    );
  }, []);

  const handleCancel = useCallback(() => {
    setState({ kind: "idle" });
  }, []);

  if (state.kind === "authenticated") {
    return <>{children}</>;
  }

  if (state.kind === "checking") {
    return (
      <div className="fixed inset-0 z-50 flex items-center justify-center bg-background">
        <LuLoaderCircle className="size-6 animate-spin text-muted-foreground" />
      </div>
    );
  }

  return (
    <LoginScreen
      state={state}
      onSignIn={handleSignIn}
      onCancel={handleCancel}
      onRetry={bootstrap}
    />
  );
}
