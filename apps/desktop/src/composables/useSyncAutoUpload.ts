/**
 * A backup that goes out on its own, while the window is open.
 *
 * There is no service and no daemon behind this: the timer lives in the interface and dies with it,
 * which is what "only while the application is open" means. That is a property rather than a
 * shortcoming — a backup agent that keeps running after somebody closed the window is a background
 * process they did not ask for, and it cannot hold the vault open to seal anything with.
 *
 * Four rules, each of which is a way this goes wrong:
 *
 * 1. **One at a time.** A tick that arrives while the last upload is still running is dropped
 *    rather than queued: a slow server must not turn into a growing queue of uploads.
 * 2. **It reads the settings every tick** rather than at the moment it was armed, so changing the
 *    interval or the address takes effect without restarting anything.
 * 3. **It does nothing while the vault is locked.** The library, the password and the passphrase are
 *    all behind the vault, so a tick before an unlock could only produce a failure — and a failure
 *    message about a backup nobody asked for yet is noise.
 * 4. **What it sends is what a manual upload sends**: the same switches, and the credentials only
 *    when the page says to sync them *and* a passphrase is actually kept to seal them with.
 *
 * A failure is said out loud; a success is not. The action has no place on screen — the person may
 * be in a terminal, in a settings page somewhere else, or away from the machine — and the rule this
 * interface follows is that a message about an action whose context is gone goes to a toast. A
 * backup that quietly worked is the one kind of message nobody needs, so it is silent.
 */
import { onBeforeUnmount, onMounted, watch } from "vue";
import { useI18n } from "vue-i18n";

import { useToast } from "@/composables/useToast";
import { describeShellError } from "@/i18n/errors";
import { usePreferencesStore } from "@/stores/preferences";
import { useSyncStore } from "@/stores/sync";
import { useVaultStore } from "@/stores/vault";
import type { SyncSelection } from "@/types/sync";

export function useSyncAutoUpload(): void {
  const { t } = useI18n();
  const { toast } = useToast();
  const preferences = usePreferencesStore();
  const sync = useSyncStore();
  const vault = useVaultStore();

  let timer: number | undefined;
  /** Whether an upload is in flight. A plain variable: nothing draws it. */
  let uploading = false;

  function clear(): void {
    if (timer !== undefined) {
      window.clearInterval(timer);
      timer = undefined;
    }
  }

  /** Arms the timer if there is anything to arm it for, and disarms it otherwise. */
  function schedule(): void {
    clear();
    if (!vault.unlocked || !preferences.syncAutoUpload || preferences.syncEndpoint.trim() === "") {
      return;
    }

    timer = window.setInterval(() => void run(), preferences.syncAutoUploadMinutes * 60_000);
  }

  async function run(): Promise<void> {
    if (uploading || !vault.unlocked) {
      return;
    }
    // Read again rather than trusted from when the timer was armed: this is one line, and it is the
    // difference between "the setting I changed a minute ago" and "the setting when I turned it on".
    if (!preferences.syncAutoUpload || preferences.syncEndpoint.trim() === "") {
      return;
    }

    uploading = true;
    try {
      // What the vault is holding, asked every tick: somebody may have forgotten the passphrase
      // since the last one, and sealing credentials there is none to seal with would write a file
      // that looks like it kept them.
      await sync.loadService();
      const selection: SyncSelection = {
        credentials: preferences.syncIncludeSecrets && sync.service?.hasPassphrase === true,
        history: false,
        knownHosts: false,
        settings: true,
      };
      const receipt = await sync.uploadToServer(selection, null);
      if (!receipt) {
        report();
      }
    } catch {
      // `uploadToServer` already records what went wrong; this is here so a failure thrown by
      // anything else cannot take the timer down with it.
      report();
    } finally {
      uploading = false;
    }
  }

  /** Says what went wrong, in the active language, once. */
  function report(): void {
    const failure = sync.failure;
    if (!failure) {
      return;
    }
    const translate = (key: string, params?: Record<string, string>): string => (params ? t(key, params) : t(key));
    toast(describeShellError(translate, failure).message, "unseenFailure");
  }

  // Re-armed whenever any of the four things it depends on changes: an unlock, the switch, the
  // interval, the address. Nothing here watches the language, because a message is built from the
  // translation in force when it is shown rather than when the timer was armed.
  watch(
    () => [vault.unlocked, preferences.syncAutoUpload, preferences.syncAutoUploadMinutes, preferences.syncEndpoint],
    () => schedule(),
  );

  onMounted(schedule);
  onBeforeUnmount(clear);
}
