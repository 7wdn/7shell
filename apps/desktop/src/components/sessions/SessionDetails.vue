<script setup lang="ts">
/**
 * What a saved session is, on hover.
 *
 * The row has room for a name and an address; everything else about a session is two clicks away in
 * the editor. This is the answer in between: the fields that identify the machine and how this
 * build would reach it, without opening anything. It is drawn beside the list rather than over it,
 * so the row it is about stays visible while it is read.
 *
 * Only what is already known is shown. A server's version, its banner and how long a connection
 * took are facts about a connection, and this card is about a saved session — a card that showed
 * them as blanks would be advertising what it does not have.
 */
import { HoverCardContent, HoverCardPortal } from "reka-ui";
import { computed } from "vue";
import { useI18n } from "vue-i18n";

import { sessionEndpoint } from "@/lib/menu/sessions";
import { groupPath } from "@/lib/sessions/groups";
import type { Session, SessionGroup } from "@/types/session";
import { authMethodById, protocolById } from "@/types/generated/protocols";

const props = defineProps<{
  readonly session: Session;
  /** The groups the session could be in, for the one that says where it sits. */
  readonly groups: readonly SessionGroup[];
}>();

const { t } = useI18n();

/** The name of the method used to sign in, as the descriptors name it. */
const authLabel = computed(() => {
  const method = authMethodById(props.session.auth.method);
  return method ? t(method.labelKey) : props.session.auth.method;
});

const protocolLabel = computed(() => {
  const protocol = protocolById(props.session.protocol);
  return protocol ? t(protocol.labelKey) : props.session.protocol;
});

/** Where the session sits: its group's path, or the fact that it is in none. */
const groupLabel = computed(() => {
  const group = props.session.groupId;
  return group !== undefined && props.groups.some((entry) => entry.id === group)
    ? groupPath(props.groups, group)
    : t("sessions.details.noGroup");
});

/**
 * The rows of the card, in the order a person reads a machine: what it is called and how to reach
 * it, then who as, then how, then where it sits and what it goes through.
 */
const rows = computed(() => [
  { key: "address", label: t("sessions.details.address"), value: sessionEndpoint(props.session), mono: true },
  { key: "host", label: t("sessions.details.host"), value: props.session.host, mono: true },
  { key: "port", label: t("sessions.details.port"), value: String(props.session.port), mono: true },
  { key: "user", label: t("sessions.details.user"), value: props.session.user, mono: true },
  { key: "protocol", label: t("sessions.details.protocol"), value: protocolLabel.value, mono: false },
  {
    key: "auth",
    label: t("sessions.details.auth"),
    value: props.session.auth.hasSecret
      ? t("sessions.details.authStored", { method: authLabel.value })
      : t("sessions.details.authMissing", { method: authLabel.value }),
    mono: false,
  },
  { key: "group", label: t("sessions.details.group"), value: groupLabel.value, mono: false },
]);

/** The two things a session can point at that are not fields of its own, when it points at them. */
const jumpLabel = computed(() => `${props.session.jump?.length ?? 0}`);
const forwardsLabel = computed(() => `${props.session.forwards?.length ?? 0}`);
</script>

<template>
  <HoverCardPortal>
    <HoverCardContent
      side="right"
      align="start"
      :side-offset="8"
      :collision-padding="8"
      class="z-50 w-80 rounded-lg border border-border bg-card p-3 shadow-xl outline-none"
      data-session-details
    >
      <p class="truncate text-sm font-medium">{{ props.session.name }}</p>

      <dl class="mt-2 space-y-1">
        <div v-for="row in rows" :key="row.key" class="flex items-baseline gap-2 text-xs">
          <dt class="w-20 shrink-0 text-muted-foreground">{{ row.label }}</dt>
          <dd class="min-w-0 flex-1 truncate" :class="row.mono ? 'font-mono' : ''" :title="row.value">
            {{ row.value }}
          </dd>
        </div>

        <div v-if="(props.session.jump?.length ?? 0) > 0" class="flex items-baseline gap-2 text-xs">
          <dt class="w-20 shrink-0 text-muted-foreground">{{ t("sessions.details.jump") }}</dt>
          <dd class="min-w-0 flex-1 truncate font-mono">{{ jumpLabel }}</dd>
        </div>
        <div v-if="(props.session.forwards?.length ?? 0) > 0" class="flex items-baseline gap-2 text-xs">
          <dt class="w-20 shrink-0 text-muted-foreground">{{ t("sessions.details.forwards") }}</dt>
          <dd class="min-w-0 flex-1 truncate font-mono">{{ forwardsLabel }}</dd>
        </div>
      </dl>
    </HoverCardContent>
  </HoverCardPortal>
</template>
