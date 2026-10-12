/**
 * What the update channel said, and what is being done about it.
 *
 * A store rather than state inside the page that installs, for two reasons. The answer arrives at
 * startup — before anybody has opened the page — and it has to survive the page being closed: a
 * check nobody is looking at is still a check. And two places read the same answer: the page that
 * installs, and the settings navigation, which is where "there is something to install" is said.
 *
 * Nothing here is remembered between runs. The answer is about the release channel as it is now, and
 * a version kept from last week would be a claim this build cannot stand behind.
 */

import { defineStore } from "pinia";
import { computed, ref } from "vue";

import { backend } from "@/lib/backend/api";
import { type ShellError, asShellError } from "@/lib/backend/errors";
import type { UpdateCheck, UpdateProgress } from "@/types/update";

export const useUpdatesStore = defineStore("updates", () => {
  /** The last answer, or nothing while the channel has not been asked in this session. */
  const answer = ref<UpdateCheck | null>(null);
  const asking = ref(false);
  const failure = ref<ShellError | null>(null);
  /** How far an installation has got, while one is running. */
  const progress = ref<UpdateProgress | null>(null);
  const installing = ref(false);

  /**
   * The release the channel offers, when it offers one.
   *
   * A computed rather than a second piece of state: "there is an update" is a reading of the answer,
   * and a copy of it could disagree with the answer it came from.
   */
  const release = computed(() => (answer.value?.kind === "available" ? answer.value.release : null));

  /**
   * Asks the channel.
   *
   * A failure is kept apart from the answer rather than folded into it: "the channel could not be
   * read" and "you are running the newest release" are different facts, and a build that showed the
   * second for the first would be telling somebody they are up to date when nobody knows.
   */
  async function check(): Promise<void> {
    if (asking.value) {
      return;
    }

    asking.value = true;
    failure.value = null;
    try {
      answer.value = await backend.updates.check();
    } catch (error) {
      answer.value = null;
      failure.value = asShellError(error, "update_check");
    } finally {
      asking.value = false;
    }
  }

  /**
   * Installs the release the last check found.
   *
   * The progress is worth drawing and the result is not: on Windows the installer takes the process
   * over, and on macOS and Linux the application restarts into the new version, so this call either
   * fails — which is reported — or the window it would have answered is already gone.
   */
  async function install(): Promise<void> {
    if (installing.value) {
      return;
    }

    installing.value = true;
    failure.value = null;
    progress.value = null;
    const stop = await backend.events.onUpdateProgress((payload) => {
      progress.value = payload;
    });

    try {
      await backend.updates.install();
    } catch (error) {
      failure.value = asShellError(error, "update_install");
    } finally {
      stop();
      installing.value = false;
    }
  }

  return { answer, asking, failure, progress, installing, release, check, install };
});
