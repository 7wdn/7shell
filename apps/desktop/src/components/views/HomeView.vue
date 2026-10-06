<script setup lang="ts">
/**
 * The home view: the surface with nothing open on it.
 *
 * Every number here comes from somewhere real: the counts are the stores the sidebar and the tabs
 * read, the capability lists are the descriptor tables, and the version and platform are what the
 * backend reported. Nothing on this screen pretends a feature exists — the honest answer to "can I
 * connect yet?" is on it.
 *
 * The shape is the one a person needs when they have nothing open: the hosts they were just using,
 * the handful of actions that start something, and the three steps of a first connection. The quick
 * start list is ranked by what was used last rather than by the library's own order, because the
 * library is already on the left and a second copy of it in the same order would be a panel that
 * answers nothing.
 */
import {
  ArrowRight,
  BadgeCheck,
  Check,
  CircleDashed,
  Download,
  FolderTree,
  KeyRound,
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
import { computed, markRaw } from "vue";
import { useI18n } from "vue-i18n";

import SessionFailure from "@/components/sessions/SessionFailure.vue";
import { useToast } from "@/composables/useToast";
import { describeShellError } from "@/i18n/errors";
import type { ShellError } from "@/lib/backend/errors";
import { claimedCapabilities, unclaimedCapabilities } from "@/lib/descriptors/claims";
import { importReceipt } from "@/lib/sessions/receipt";
import { useAppStore } from "@/stores/app";
import { usePreferencesStore } from "@/stores/preferences";
import { useRecentSessionsStore } from "@/stores/recentSessions";
import { useSessionEditorStore } from "@/stores/sessionEditor";
import { useSessionsStore } from "@/stores/sessions";
import { useTerminalsStore } from "@/stores/terminals";
import { AUTH_METHODS } from "@/types/generated/protocols";
import type { Session } from "@/types/session";

const { t } = useI18n();
const app = useAppStore();
const editor = useSessionEditorStore();
const library = useSessionsStore();
const preferences = usePreferencesStore();
const recents = useRecentSessionsStore();
const terminals = useTerminalsStore();
const { toast } = useToast();

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

/** What the window is made of, as numbers: saved things, and what this build claims it can do. */
const tiles = computed(() => [
  { key: "sessions", icon: markRaw(Server), label: t("home.tile.sessions"), value: library.sessions.length },
  { key: "groups", icon: markRaw(FolderTree), label: t("home.tile.groups"), value: library.groups.length },
  { key: "protocols", icon: markRaw(Network), label: t("home.tile.protocols"), value: app.info?.protocolCount ?? 0 },
  { key: "capabilities", icon: markRaw(BadgeCheck), label: t("home.tile.capabilities"), value: claimed.value.length },
  { key: "authMethods", icon: markRaw(KeyRound), label: t("home.tile.authMethods"), value: AUTH_METHODS.length },
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

const buildKind = computed(() => {
  if (!app.info) {
    return "";
  }
  return app.info.debugBuild ? t("home.about.debugBuild") : t("home.about.releaseBuild");
});

/** The line under everything: what this is, which version, and which machine it is running on. */
const stamp = computed(() =>
  ["7Shell", app.info ? `v${app.info.version}` : "", app.platform].filter(Boolean).join(" · "),
);

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}
</script>

<template>
  <div data-home class="mx-auto flex min-h-full w-full min-w-0 max-w-5xl flex-col justify-center gap-6 px-8 py-10">
    <section class="grid min-w-0 grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-5">
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
                <span class="block truncate font-mono text-[10px] text-muted-foreground">{{ endpoint(session) }}</span>
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

    <div class="grid min-w-0 grid-cols-1 gap-4 lg:grid-cols-2">
      <section class="min-w-0 rounded-lg border border-border bg-card p-4">
        <h2 class="flex items-center gap-2 text-sm font-medium">
          <ShieldCheck :size="15" class="text-muted-foreground" />
          {{ t("home.about.title") }}
        </h2>
        <dl v-if="app.info" class="mt-3 grid grid-cols-[auto_1fr] gap-x-6 gap-y-2 text-sm">
          <dt class="text-muted-foreground">{{ t("home.about.version") }}</dt>
          <dd class="font-mono">{{ app.info.version }}</dd>

          <dt class="text-muted-foreground">{{ t("home.about.platform") }}</dt>
          <dd class="font-mono">{{ app.info.os }} {{ app.info.arch }}</dd>

          <dt class="text-muted-foreground">{{ t("home.about.build") }}</dt>
          <dd>{{ buildKind }}</dd>

          <dt class="text-muted-foreground">{{ t("home.about.descriptorSchema") }}</dt>
          <dd class="font-mono">{{ app.info.descriptorSchemaVersion }}</dd>
        </dl>
        <p v-else class="mt-3 text-sm text-muted-foreground">{{ t("common.status.loading") }}</p>
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
    </div>

    <div
      class="mt-1 flex min-w-0 flex-col items-center gap-1 text-center text-[11px] leading-5 text-muted-foreground/70"
    >
      <p>{{ stamp }}</p>
      <p class="max-w-2xl">{{ t("common.notice.early") }}</p>
    </div>
  </div>
</template>
