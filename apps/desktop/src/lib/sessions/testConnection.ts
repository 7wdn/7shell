/**
 * Running a connection test and saying what it answered.
 *
 * The answer is a toast: it appears at the foot of the window and leaves on its own, because a test
 * is an action whose result does not belong to any one field. The two host key decisions are the
 * exception inside the message — an unseen key offers "trust and test again" as the toast's one
 * action, and a changed key is a hard stop with both fingerprints in the text and no action at all.
 *
 * One helper for both callers: the editor tests a draft with whatever password was typed, the row's
 * button tests the saved session as it is stored. Neither keeps the result anywhere, so the next
 * test replaces the last message.
 */
import { useToast } from "@/composables/useToast";
import { i18n } from "@/i18n";
import { describeShellError } from "@/i18n/errors";
import { backend } from "@/lib/backend/api";
import { asShellError } from "@/lib/backend/errors";
import type { PresentedHostKey, SecretInput, Session } from "@/types/session";

function translate(key: string, params?: Record<string, string>): string {
  return params ? i18n.global.t(key, params) : i18n.global.t(key);
}

/** The code beneath a failure is what somebody quotes back; it goes on the toast's second line. */
function failureText(error: Parameters<typeof asShellError>[0], operation: string): string {
  const refusal = asShellError(error, operation);
  const described = describeShellError(translate, refusal);
  return `${described.message}\n${described.code}`;
}

export async function runConnectionTest(session: Session, secret?: SecretInput): Promise<void> {
  const { toast } = useToast();
  try {
    const outcome = await backend.sessions.test(session, secret);
    if (outcome.kind === "connected") {
      const line = outcome.authenticated
        ? translate("sessions.connectionTest.signedIn", { ms: String(outcome.latencyMs) })
        : translate("sessions.connectionTest.reachable", { ms: String(outcome.latencyMs) });
      toast(
        outcome.fingerprint
          ? `${line}\n${translate("sessions.connectionTest.fingerprint", { value: outcome.fingerprint })}`
          : line,
        "result",
      );
      return;
    }

    if (outcome.kind === "hostKeyUnknown") {
      const key = outcome.key;
      toast(
        [translate("sessions.connectionTest.unknownTitle"), `${key.host}:${key.port}`, key.fingerprint].join("\n"),
        "refusal",
        {
          label: translate("sessions.connectionTest.trust"),
          onClick: () => {
            void trustThenTest(key, session, secret);
          },
        },
      );
      return;
    }

    // A changed key has no action: trusting it is not a thing that toast can offer.
    toast(
      [
        translate("sessions.connectionTest.mismatchTitle"),
        `${outcome.key.host}:${outcome.key.port}`,
        translate("sessions.connectionTest.mismatchBody"),
        translate("sessions.connectionTest.trustedFingerprint", { value: outcome.trusted.fingerprint }),
        translate("sessions.connectionTest.presentedFingerprint", { value: outcome.key.fingerprint }),
      ].join("\n"),
      "failure",
    );
  } catch (error) {
    toast(failureText(error, "session_test"), "failure");
  }
}

async function trustThenTest(key: PresentedHostKey, session: Session, secret?: SecretInput): Promise<void> {
  const { toast } = useToast();
  try {
    await backend.knownHosts.trust(key);
  } catch (error) {
    toast(failureText(error, "known_host_trust"), "failure");
    return;
  }
  await runConnectionTest(session, secret);
}
