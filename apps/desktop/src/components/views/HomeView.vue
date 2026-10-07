<script setup lang="ts">
/**
 * The home view: the surface with nothing open on it.
 *
 * Every number here comes from somewhere real: the counts are the stores the sidebar and the tabs
 * read, the capability lists are the descriptor tables, and the version and platform are what the
 * backend reported. Nothing on this screen pretends a feature exists — the honest answer to "can I
 * connect yet?" is on it.
 *
 * It is two pages rather than one page with parts that can be put away. Somebody who has never
 * connected anything wants to know what this is and how a first connection goes; somebody who has
 * wants their own hosts and what they were doing. The parts belong together — the counters and the
 * host names are one answer, not two — so which page is drawn is one variable, chosen in the
 * settings page, rather than a row of conditions spread through the template.
 */
import {
  Activity,
  ArrowRight,
  Check,
  CircleDashed,
  Download,
  FolderTree,
  History,
  ListChecks,
  LoaderCircle,
  MousePointerClick,
  Network,
  Plus,
  Server,
  Settings,
  ShieldCheck,
  Sparkles,
  SquareTerminal,
  Terminal,
  X,
} from "@lucide/vue";
import { computed, markRaw, onMounted } from "vue";
import { useI18n } from "vue-i18n";

import SessionFailure from "@/components/sessions/SessionFailure.vue";
import { useToast } from "@/composables/useToast";
import { describeShellError } from "@/i18n/errors";
import type { ShellError } from "@/lib/backend/errors";
import { claimedCapabilities, unclaimedCapabilities } from "@/lib/descriptors/claims";
import { importReceipt } from "@/lib/sessions/receipt";
import { useAppStore } from "@/stores/app";
import { useCommandsStore } from "@/stores/commands";
import { usePreferencesStore } from "@/stores/preferences";
import { useRecentSessionsStore } from "@/stores/recentSessions";
import { useSessionEditorStore } from "@/stores/sessionEditor";
import { useSessionsStore } from "@/stores/sessions";
import { useTerminalsStore } from "@/stores/terminals";
import type { Session } from "@/types/session";

const { t } = useI18n();
const app = useAppStore();
const editor = useSessionEditorStore();
const library = useSessionsStore();
const preferences = usePreferencesStore();
const recents = useRecentSessionsStore();
const terminals = useTerminalsStore();
const commands = useCommandsStore();
const { toast } = useToast();

// What was run lives in the vault rather than in a file, so it is read when the page appears — and
// the page is only ever drawn with the vault unlocked, which is what the shell means.
onMounted(() => void commands.load());

function endpoint(session: Session): string {
  return `${session.user}@${session.host}:${session.port}`;
}

/**
 * What this build can do, and what nothing claims yet.
 *
 * From `lib/descriptors/claims`, which asks every descriptor — a capability an authentication
 * method claims is as real as one a protocol claims, and asking only about protocols is what made
 * this page report stored secrets and key files as missing.
 */
const claimed = computed(() => claimedCapabilities());
const unclaimed = computed(() => unclaimedCapabilities());

/**
 * What the library holds, in three numbers, across the top of the working page.
 *
 * Three rather than five because a row of five is a table and a row of three is a heading: the two
 * the row gave up — what this build claims and how many ways it can authenticate — are properties of
 * the build rather than of somebody's library, and they are on the introduction page, where what
 * this build is has a place of its own.
 */
const tiles = computed(() => [
  { key: "sessions", icon: markRaw(Server), label: t("home.tile.sessions"), value: library.sessions.length },
  { key: "groups", icon: markRaw(FolderTree), label: t("home.tile.groups"), value: library.groups.length },
  { key: "protocols", icon: markRaw(Network), label: t("home.tile.protocols"), value: app.info?.protocolCount ?? 0 },
]);

/** The commands sent most recently, newest first — the working page's third panel. */
const recentCommands = computed(() => commands.history.slice(0, 6));

/** When a command was sent, in the reader's own convention. */
function sentAt(at: number): string {
  return new Date(at * 1000).toLocaleString();
}

/**
 * The four things this build does, for the introduction's cards.
 *
 * Written here rather than derived from the descriptors, and that is deliberate: the descriptors say
 * what a *protocol* can do, and these four are what this application is for — which is a sentence
 * somebody wrote, not a table somebody filled in. What the descriptors say is the card underneath,
 * where the lists are real and the ones nothing claims are on it too.
 */
const features = computed(() => [
  {
    key: "sessions",
    icon: markRaw(Server),
    title: t("home.feature.sessions.title"),
    body: t("home.feature.sessions.body"),
  },
  {
    key: "terminal",
    icon: markRaw(SquareTerminal),
    title: t("home.feature.terminal.title"),
    body: t("home.feature.terminal.body"),
  },
  {
    key: "files",
    icon: markRaw(FolderTree),
    title: t("home.feature.files.title"),
    body: t("home.feature.files.body"),
  },
  {
    key: "monitor",
    icon: markRaw(Activity),
    title: t("home.feature.monitor.title"),
    body: t("home.feature.monitor.body"),
  },
]);

/**
 * The sessions to offer, most recently used first.
 *
 * Filled out from the library when fewer than the cap have been used — a fresh install has nothing
 * remembered, and a card that stayed empty until somebody had opened something would teach nothing.
 */
const quick = computed(() => recents.ranked(library.sessions));

/**
 * What the shortcuts do.
 *
 * Four actions rather than everything the application can do: these are the ones that start
 * something, and every one of them is reachable from here without the sidebar, which is the point —
 * somebody who put the sidebar away still has to be able to make a session.
 */
const actions = computed(() => [
  { key: "new", icon: markRaw(Plus), label: t("sessions.list.add"), run: () => editor.startNew() },
  { key: "import", icon: markRaw(Download), label: t("sessions.list.import"), run: () => void runImport() },
  { key: "batch", icon: markRaw(ListChecks), label: t("batch.title"), run: () => terminals.openBatch() },
  { key: "settings", icon: markRaw(Settings), label: t("settings.title"), run: () => terminals.openSettings() },
]);

/**
 * Reads a file the user picks and says what it added.
 *
 * The receipt is a toast here as it is in the sidebar, but a *refusal* is not left to the list's own
 * box the way the sidebar leaves it: that box is in the sidebar, and this page's whole reason for
 * having an import shortcut is that the sidebar may be put away. So a failure with the sidebar on
 * screen stays where the library is drawn, and one without it is said out loud instead of nowhere.
 */
async function runImport(): Promise<void> {
  const summary = await library.importLibrary(t("sessions.list.importTitle"));
  if (summary) {
    toast(importReceipt(summary, t), summary.missingKeys + summary.droppedHops > 0 ? "result" : "confirmation");
    return;
  }
  if (!preferences.sidebarOpen && library.failure) {
    toast(describeShellError(translate, library.failure).message, "failure");
  }
}

function isConnecting(session: Session): boolean {
  return terminals.connectingSessionIds.includes(session.id);
}

function failureOf(session: Session): ShellError | undefined {
  return terminals.failures[session.id];
}

/** The line under everything: what this is, which version, and which machine it is running on. */
const stamp = computed(() =>
  ["7Shell", app.info ? `v${app.info.version}` : "", app.platform].filter(Boolean).join(" · "),
);

const working = computed(() => preferences.homeLayout === "workspace");

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}
</script>

<template>
  <div data-home class="mx-auto flex min-h-full w-full min-w-0 max-w-5xl flex-col justify-center gap-6 px-8 py-10">
    <!--
      The working page: what the library holds, the hosts used last, the actions that start
      something, and what was run. In that order, and the order is the argument — the numbers first
      because they are the answer to "is anything here", then the two ways in, then history.
    -->
    <template v-if="working">
      <section class="grid min-w-0 grid-cols-2 gap-3 sm:grid-cols-3">
        <div
          v-for="tile in tiles"
          :key="tile.key"
          class="min-w-0 overflow-hidden rounded-lg border border-border bg-card px-4 py-3"
        >
          <div class="flex min-w-0 items-center gap-2 text-xs text-muted-foreground">
            <component :is="tile.icon" :size="14" class="shrink-0" />
            <span class="min-w-0 truncate">{{ tile.label }}</span>
          </div>
          <div class="mt-2 font-mono text-2xl leading-tight">{{ tile.value }}</div>
        </div>
      </section>

      <div class="grid min-w-0 grid-cols-1 gap-4 lg:grid-cols-[minmax(0,1.2fr)_minmax(0,0.8fr)]">
        <!--
          The hosts somebody was just on. A row opens a terminal, exactly as the sidebar's rows do —
          the same store action, so a host key still to be decided, a cancelled attempt and a failed
          one all behave the same way wherever the click was made.
        -->
        <section class="min-w-0 overflow-hidden rounded-lg border border-border bg-card">
          <div class="flex items-center justify-between border-b border-border px-4 py-3">
            <h2 class="text-sm font-medium">{{ t("home.quick.title") }}</h2>
          </div>
          <div class="divide-y divide-border">
            <div
              v-for="session in quick"
              :key="session.id"
              :data-quick-session="session.id"
              class="flex min-w-0 items-center gap-1 px-2 transition-colors hover:bg-accent/40"
            >
              <button
                type="button"
                class="flex min-w-0 flex-1 items-center gap-3 rounded-md px-2 py-2.5 text-left"
                :title="t('sessions.list.open', { name: session.name })"
                @click="terminals.open(session)"
              >
                <Server :size="14" class="shrink-0 text-muted-foreground" />
                <span class="min-w-0 flex-1">
                  <span class="block truncate text-sm">{{ session.name }}</span>
                  <span class="block truncate font-mono text-[11px] text-muted-foreground">{{
                    endpoint(session)
                  }}</span>
                </span>
                <SquareTerminal :size="15" class="shrink-0 text-muted-foreground" />
              </button>

              <!-- What did not connect leaves its mark here too: nothing came up, so there is no tab
                 to draw it in and this row is where the click was made. -->
              <SessionFailure
                v-if="failureOf(session)"
                :failure="failureOf(session)!"
                :name="session.name"
                @dismiss="terminals.forgetFailure(session.id)"
              />

              <LoaderCircle
                v-if="isConnecting(session)"
                :size="12"
                class="shrink-0 animate-spin text-muted-foreground"
                :aria-label="t('sessions.list.connecting', { name: session.name })"
              />
              <button
                v-if="isConnecting(session)"
                type="button"
                class="grid size-5 shrink-0 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                :title="t('sessions.list.cancelConnecting', { name: session.name })"
                :aria-label="t('sessions.list.cancelConnecting', { name: session.name })"
                @click="terminals.cancelOpen(session.id)"
              >
                <X :size="12" />
              </button>
            </div>

            <p v-if="quick.length === 0" class="px-4 py-8 text-sm text-muted-foreground">
              {{ t("sessions.list.empty") }}
            </p>
          </div>
        </section>

        <section class="min-w-0 overflow-hidden rounded-lg border border-border bg-card">
          <div class="border-b border-border px-4 py-3">
            <h2 class="text-sm font-medium">{{ t("home.actions.title") }}</h2>
          </div>
          <div class="grid min-w-0 gap-1 p-2">
            <button
              v-for="action in actions"
              :key="action.key"
              type="button"
              class="flex min-w-0 items-center gap-3 rounded-md px-3 py-2 text-left text-sm transition-colors hover:bg-accent/60"
              @click="action.run()"
            >
              <component :is="action.icon" :size="16" class="shrink-0 text-muted-foreground" />
              <span class="min-w-0 flex-1 truncate">{{ action.label }}</span>
              <ArrowRight :size="14" class="shrink-0 text-muted-foreground/60" />
            </button>

            <p
              class="mt-2 flex min-w-0 items-start gap-2 rounded-md bg-muted/40 px-3 py-2 text-xs leading-5 text-muted-foreground"
            >
              <MousePointerClick :size="14" class="mt-0.5 shrink-0" />
              <span class="min-w-0">{{ t("home.actions.tip") }}</span>
            </p>
          </div>
        </section>
      </div>

      <!--
        What was run, newest first — the panel that is about what somebody *did* rather than about
        what they have. Read from the command library, which keeps it in the vault rather than in a
        file, so an empty answer here is a real one: nothing has been sent yet.
      -->
      <section class="min-w-0 overflow-hidden rounded-lg border border-border bg-card">
        <div class="flex items-center gap-2 border-b border-border px-4 py-3">
          <History :size="15" class="text-muted-foreground" />
          <h2 class="text-sm font-medium">{{ t("commands.history") }}</h2>
        </div>
        <ul v-if="recentCommands.length > 0" class="divide-y divide-border">
          <li
            v-for="(entry, index) in recentCommands"
            :key="`${entry.at}-${index}`"
            class="flex min-w-0 items-baseline gap-3 px-4 py-2"
          >
            <code class="min-w-0 flex-1 truncate font-mono text-xs">{{ entry.command }}</code>
            <span class="shrink-0 text-[11px] text-muted-foreground">{{ sentAt(entry.at) }}</span>
          </li>
        </ul>
        <p v-else class="px-4 py-6 text-sm text-muted-foreground">{{ t("commands.historyEmpty") }}</p>
      </section>
    </template>

    <!--
      The introduction: what this is, what it does, how a first connection goes, and what this build
      cannot do yet. A page for somebody who has not connected anything, which is why nothing on it
      names a host or counts one.
    -->
    <template v-else>
      <section class="min-w-0 rounded-lg border border-border bg-card px-8 py-9">
        <span
          class="inline-flex items-center gap-2 rounded-md border border-border bg-muted/30 px-2.5 py-1 text-xs text-muted-foreground"
        >
          <SquareTerminal :size="13" />
          {{ t("home.intro.badge") }}
        </span>
        <h1 class="mt-4 text-2xl leading-tight font-medium">{{ t("home.intro.headline") }}</h1>
        <p class="mt-3 max-w-2xl text-sm leading-6 text-muted-foreground">{{ t("home.intro.body") }}</p>
        <div class="mt-6 flex flex-wrap gap-2">
          <button
            type="button"
            class="flex items-center gap-1.5 rounded-md bg-primary px-3 py-1.5 text-sm text-primary-foreground transition-colors hover:bg-primary/90"
            @click="editor.startNew()"
          >
            <Plus :size="14" />
            {{ t("sessions.list.add") }}
            <ArrowRight :size="14" />
          </button>
          <button
            type="button"
            class="flex items-center gap-1.5 rounded-md border border-border px-3 py-1.5 text-sm transition-colors hover:bg-accent"
            @click="runImport()"
          >
            <Download :size="14" />
            {{ t("sessions.list.import") }}
          </button>
        </div>
      </section>

      <section class="grid min-w-0 gap-3 sm:grid-cols-2 lg:grid-cols-4">
        <div v-for="feature in features" :key="feature.key" class="min-w-0 rounded-lg border border-border bg-card p-4">
          <component :is="feature.icon" :size="16" class="text-muted-foreground" />
          <h2 class="mt-3 text-sm font-medium">{{ feature.title }}</h2>
          <p class="mt-1 text-xs leading-5 text-muted-foreground">{{ feature.body }}</p>
        </div>
      </section>

      <!--
        The first connection, as the three decisions it actually is. Kept on the page rather than
        moved into a manual: the second step is the one nobody expects, and it is the one this
        application will stop a connection over.
      -->
      <section class="min-w-0 rounded-lg border border-border bg-card p-5">
        <h2 class="flex items-center gap-2 text-sm font-medium">
          <Terminal :size="15" class="text-muted-foreground" />
          {{ t("home.start.title") }}
        </h2>
        <ol class="mt-4 grid min-w-0 gap-4 sm:grid-cols-3">
          <li
            v-for="(step, index) in [
              t('home.start.stepSession'),
              t('home.start.stepHostKey'),
              t('home.start.stepTerminal'),
            ]"
            :key="index"
            class="flex min-w-0 gap-3"
          >
            <span
              class="grid size-6 shrink-0 place-items-center rounded-full border border-border font-mono text-xs text-muted-foreground"
              >{{ index + 1 }}</span
            >
            <span class="min-w-0 text-sm leading-5">{{ step }}</span>
          </li>
        </ol>
        <p
          class="mt-4 rounded-md border border-dashed border-border px-3 py-2 text-xs leading-relaxed text-muted-foreground"
        >
          {{ t("home.start.pending") }}
        </p>
      </section>

      <section class="min-w-0 rounded-lg border border-border bg-card p-4">
        <h2 class="flex items-center gap-2 text-sm font-medium">
          <Sparkles :size="15" class="text-muted-foreground" />
          {{ t("home.status.title") }}
        </h2>
        <p class="mt-1 text-xs text-muted-foreground">{{ t("home.status.note") }}</p>

        <p class="mt-3 text-xs text-muted-foreground">{{ t("home.status.claimed") }}</p>
        <ul class="mt-1.5 flex flex-wrap gap-1.5">
          <li
            v-for="capability in claimed"
            :key="capability.key"
            class="flex items-center gap-1.5 rounded-md border border-border px-2 py-1 text-xs"
          >
            <Check :size="12" class="text-success" />
            {{ t(capability.labelKey) }}
          </li>
        </ul>

        <p class="mt-3 text-xs text-muted-foreground">{{ t("home.status.unclaimed") }}</p>
        <ul class="mt-1.5 flex flex-wrap gap-1.5">
          <li
            v-for="capability in unclaimed"
            :key="capability.key"
            class="flex items-center gap-1.5 rounded-md border border-dashed border-border px-2 py-1 text-xs text-muted-foreground"
          >
            <CircleDashed :size="12" />
            {{ t(capability.labelKey) }}
          </li>
        </ul>
      </section>

      <!--
        The one promise this page makes and the other page keeps: what is *not* here. Somebody
        arriving from other client software has a right to know that this page shows nothing about
        their hosts — and where the choice between the two pages is made.
      -->
      <div
        class="flex min-w-0 flex-wrap items-center justify-between gap-3 rounded-lg border border-border bg-muted/20 px-4 py-2.5"
      >
        <p class="flex min-w-0 items-center gap-2 text-xs text-muted-foreground">
          <ShieldCheck :size="14" class="shrink-0" />
          <span class="min-w-0">{{ t("home.intro.notice") }}</span>
        </p>
        <button
          type="button"
          class="shrink-0 rounded-md px-2 py-1 text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          @click="terminals.openSettings()"
        >
          {{ t("home.intro.settings") }}
        </button>
      </div>
    </template>

    <div
      class="mt-1 flex min-w-0 flex-col items-center gap-1 text-center text-[11px] leading-5 text-muted-foreground/70"
    >
      <p>{{ stamp }}</p>
      <p class="max-w-2xl">{{ t("common.notice.early") }}</p>
    </div>
  </div>
</template>
