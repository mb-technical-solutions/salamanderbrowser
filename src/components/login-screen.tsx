"use client";

import { LuLoaderCircle, LuShieldCheck } from "react-icons/lu";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";

export type LoginScreenState =
  | { kind: "idle" }
  | { kind: "waitingForBrowser" }
  | { kind: "error"; message: string };

interface LoginScreenProps {
  state: LoginScreenState;
  onSignIn: () => void;
  onCancel: () => void;
  onRetry: () => void;
}

/** Full-viewport gate shown by `AuthGateProvider` whenever the app isn't
 * authenticated. Replaces the entire app shell rather than overlaying it. */
export function LoginScreen({
  state,
  onSignIn,
  onCancel,
  onRetry,
}: LoginScreenProps) {
  const { t } = useTranslation();

  return (
    <div className="fixed inset-0 z-50 flex flex-col items-center justify-center gap-6 bg-background px-6 text-center">
      <LuShieldCheck className="size-12 text-muted-foreground" />

      {state.kind === "error" ? (
        <>
          <div className="space-y-2">
            <h1 className="text-lg font-semibold text-foreground">
              {t("auth.errorTitle")}
            </h1>
            <p className="max-w-sm text-sm text-muted-foreground">
              {state.message}
            </p>
          </div>
          <Button onClick={onRetry}>{t("auth.retry")}</Button>
        </>
      ) : state.kind === "waitingForBrowser" ? (
        <>
          <div className="space-y-2">
            <h1 className="text-lg font-semibold text-foreground">
              {t("auth.signInTitle")}
            </h1>
            <p className="max-w-sm text-sm text-muted-foreground">
              {t("auth.waitingForBrowser")}
            </p>
          </div>
          <LuLoaderCircle className="size-5 animate-spin text-muted-foreground" />
          <Button variant="ghost" onClick={onCancel}>
            {t("auth.cancel")}
          </Button>
        </>
      ) : (
        <>
          <div className="space-y-2">
            <h1 className="text-lg font-semibold text-foreground">
              {t("auth.signInTitle")}
            </h1>
            <p className="max-w-sm text-sm text-muted-foreground">
              {t("auth.signInTagline")}
            </p>
          </div>
          <Button onClick={onSignIn}>{t("auth.signInButton")}</Button>
        </>
      )}
    </div>
  );
}
