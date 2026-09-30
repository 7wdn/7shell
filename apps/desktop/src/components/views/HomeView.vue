<script setup lang="ts">
/**
 * The home view.
 *
 * Every number here comes from somewhere real: the saved and open counts are the stores the
 * sidebar and the tabs read, the protocol and capability counts are the descriptor tables, the
 * version and platform are what the backend reported, and the lists of what is and is not claimed
 * are the descriptor vocabulary itself. Nothing on this screen pretends a feature exists — the
 * honest answer to "can I connect yet?" is on it.
 */
import { Check, CircleDashed, KeyRound, ShieldCheck, Terminal } from "@lucide/vue";
import { computed } from "vue";
import { useI18n } from "vue-i18n";

import { useAppStore } from "@/stores/app";
import { useSessionsStore } from "@/stores/sessions";
import { useTerminalsStore } from "@/stores/terminals";
import { AUTH_METHODS, CAPABILITIES, PROTOCOLS, protocolSupports } from "@/types/generated/protocols";

const { t } = useI18n();
const app = useAppStore();
const library = useSessionsStore();
const terminals = useTerminalsStore();

const protocols = computed(() => PROTOCOLS);
const authMethods = computed(() => AUTH_METHODS);

/** A terminal that ended is not open, and a tab whose open failed never held one. */
const openTerminals = computed(() => terminals.tabs.filter((tab) => tab.status === "open").length);

/** The capabilities SSH declares, and the ones still sitting in the catalog unclaimed. */
const claimedCapabilities = computed(() =>
  CAPABILITIES.filter((capability) =>
    protocols.value.some((protocol) => protocolSupports(protocol.id, capability.key)),
  ),
);
const unclaimedCapabilities = computed(() =>
  CAPABILITIES.filter(
    (capability) => !protocols.value.some((protocol) => protocolSupports(protocol.id, capability.key)),
  ),
);

const tiles = computed(() => [
  { key: "sessions", label: t("home.tile.sessions"), value: library.sessions.length },
  { key: "terminals", label: t("home.tile.terminals"), value: openTerminals.value },
  { key: "protocols", label: t("home.tile.protocols"), value: app.info?.protocolCount ?? 0 },
  { key: "capabilities", label: t("home.tile.capabilities"), value: claimedCapabilities.value.length },
  { key: "authMethods", label: t("home.tile.authMethods"), value: authMethods.value.length },
]);

const buildKind = computed(() => {
  if (!app.info) {
    return "";
  }
  return app.info.debugBuild ? t("home.about.debugBuild") : t("home.about.releaseBuild");
});
</script>

<template>
  <div class="mx-auto flex max-w-5xl flex-col gap-4 p-5">
    <section class="grid gap-3 sm:grid-cols-2 lg:grid-cols-5">
      <div v-for="tile in tiles" :key="tile.key" class="rounded-lg border border-border bg-card px-4 py-3">
        <p class="text-xs text-muted-foreground">{{ tile.label }}</p>
        <p class="mt-1 font-mono text-2xl leading-tight">{{ tile.value }}</p>
      </div>
    </section>

    <section class="grid gap-4 lg:grid-cols-2">
      <article class="rounded-lg border border-border bg-card p-4">
        <h2 class="flex items-center gap-2 text-sm font-medium">
          <Terminal :size="15" class="text-muted-foreground" />
          {{ t("home.start.title") }}
        </h2>
        <ol class="mt-3 space-y-2 text-sm">
          <li class="flex gap-2">
            <span class="mt-0.5 font-mono text-xs text-muted-foreground">1</span>
            <span>{{ t("home.start.stepSession") }}</span>
          </li>
          <li class="flex gap-2">
            <span class="mt-0.5 font-mono text-xs text-muted-foreground">2</span>
            <span>{{ t("home.start.stepHostKey") }}</span>
          </li>
          <li class="flex gap-2">
            <span class="mt-0.5 font-mono text-xs text-muted-foreground">3</span>
            <span>{{ t("home.start.stepTerminal") }}</span>
          </li>
        </ol>
        <p
          class="mt-3 rounded-md border border-dashed border-border px-3 py-2 text-xs leading-relaxed text-muted-foreground"
        >
          {{ t("home.start.pending") }}
        </p>
      </article>

      <article class="rounded-lg border border-border bg-card p-4">
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
      </article>
    </section>

    <section class="rounded-lg border border-border bg-card p-4">
      <h2 class="flex items-center gap-2 text-sm font-medium">
        <KeyRound :size="15" class="text-muted-foreground" />
        {{ t("home.status.title") }}
      </h2>
      <p class="mt-1 text-xs text-muted-foreground">{{ t("home.status.note") }}</p>

      <div class="mt-3 grid gap-4 lg:grid-cols-2">
        <div>
          <p class="mb-2 text-xs text-muted-foreground">{{ t("home.status.claimed") }}</p>
          <ul class="flex flex-wrap gap-1.5">
            <li
              v-for="capability in claimedCapabilities"
              :key="capability.key"
              class="flex items-center gap-1.5 rounded-md border border-border px-2 py-1 text-xs"
            >
              <Check :size="12" class="text-success" />
              {{ t(capability.labelKey) }}
            </li>
          </ul>
        </div>

        <div>
          <p class="mb-2 text-xs text-muted-foreground">{{ t("home.status.unclaimed") }}</p>
          <ul class="flex flex-wrap gap-1.5">
            <li
              v-for="capability in unclaimedCapabilities"
              :key="capability.key"
              class="flex items-center gap-1.5 rounded-md border border-dashed border-border px-2 py-1 text-xs text-muted-foreground"
            >
              <CircleDashed :size="12" />
              {{ t(capability.labelKey) }}
            </li>
          </ul>
        </div>
      </div>
    </section>

    <p class="px-1 pb-1 text-[11px] leading-relaxed text-muted-foreground">{{ t("common.notice.early") }}</p>
  </div>
</template>
