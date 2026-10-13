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

/**
 * How long the startup check waits after the window is up.
 *
 * Long enough that the window, the vault and the first session are all somebody else's problem by the
 * time it runs, short enough that a release published while somebody was away is on their screen
 * before they have finished reading the tab bar.
 */
export const STARTUP_CHECK_DELAY_MS = 3_000;

/**
 * How long a failed check waits before trying again, and how many times it tries.
 *
 * The first is short because a network that is not up yet is usually up a moment later; the last is
 * fifteen minutes, which is about as long as somebody leaves an application open before looking at it
 * again. After the last one the channel is simply unread until the next run or a manual check.
 */
export const RETRY_DELAYS_MS = [60_000, 300_000, 900_000] as const;

/** Waits. A timer rather than a sleeping loop: nothing here is holding anything up. */
function wait(milliseconds: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

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

  /**
   * Asks the channel a moment after the window is up, and asks again when it could not be read.
   *
   * Late on purpose. This is the one request the application makes on its own, and it is the one
   * request nobody is waiting for: a window that appears only after a release server has answered is
   * a window that takes as long to open as the network does, which is exactly what a slow link — or a
   * blocked one — turns a fast application into. Waiting also means the first frame, the vault and
   * everything else the person is looking at have already happened.
   *
   * Asked again because the answer is what the mark on the title bar is made of, and a check that
   * failed because the network was not up yet is not an answer. The waits are the ones a person
   * cannot notice: a minute, then five, then fifteen, after which the mark is simply absent until
   * somebody starts the application again or asks for a check themselves.
   */
  async function checkAfterStartup(): Promise<void> {
    await wait(STARTUP_CHECK_DELAY_MS);
    await check();

    for (const delay of RETRY_DELAYS_MS) {
      // Only a failure is worth asking about again: an answer — even "nothing newer is published" —
      // is the channel having been read.
      if (failure.value === null) {
        return;
      }
      await wait(delay);
      await check();
    }
  }

  return { answer, asking, failure, progress, installing, release, check, checkAfterStartup, install };
});
