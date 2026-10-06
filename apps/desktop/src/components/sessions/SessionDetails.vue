<script setup lang="ts">
/**
 * What a saved session is, on hover.
 *
 * The row has room for a name and an address; everything else about a session is two clicks away in
 * the editor. This is the answer in between: the fields that identify the machine and how this build
 * would reach it, without opening anything.
 *
 * It is a bubble rather than a card floating nowhere — a label/value list with a pointer aimed at
 * the row it is about, arriving the moment the pointer stops on that row and gone the moment it
 * leaves. The reference draws it the same way, and the pointer is what makes it read as "about
 * *this* row" rather than as a panel that happened to appear.
 *
 * Only what is already known is shown. A server's version, its banner and how long a connection
 * took are facts about a connection, and this is about a saved session — a card that showed them
 * as blanks would be advertising what it does not have.
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
 * The rows of the bubble, in the order a person reads a machine: what it is called and how to reach
 * it, then who as, then how, then where it sits — and what it goes through, when it goes through
 * anything.
 */
const rows = computed(() => {
  const list = [
    { key: "name", label: t("sessions.details.name"), value: props.session.name },
    { key: "address", label: t("sessions.details.address"), value: sessionEndpoint(props.session) },
    { key: "host", label: t("sessions.details.host"), value: props.session.host },
    { key: "port", label: t("sessions.details.port"), value: String(props.session.port) },
    { key: "user", label: t("sessions.details.user"), value: props.session.user },
    { key: "protocol", label: t("sessions.details.protocol"), value: protocolLabel.value },
    {
      key: "auth",
      label: t("sessions.details.auth"),
      value: props.session.auth.hasSecret
        ? t("sessions.details.authStored", { method: authLabel.value })
        : t("sessions.details.authMissing", { method: authLabel.value }),
    },
    { key: "group", label: t("sessions.details.group"), value: groupLabel.value },
  ];

  const jump = props.session.jump?.length ?? 0;
  if (jump > 0) {
    list.push({ key: "jump", label: t("sessions.details.jump"), value: String(jump) });
  }
  const forwards = props.session.forwards?.length ?? 0;
  if (forwards > 0) {
    list.push({ key: "forwards", label: t("sessions.details.forwards"), value: String(forwards) });
  }
  return list;
});
</script>

<template>
  <HoverCardPortal>
    <HoverCardContent
      side="right"
      align="start"
      :side-offset="10"
      :collision-padding="8"
      class="relative z-50 w-max min-w-40 max-w-[min(28rem,calc(100vw-24px))] rounded-md border border-border bg-card p-2 shadow-lg outline-none"
      data-session-details
    >
      <div class="space-y-1">
        <div
          v-for="row in rows"
          :key="row.key"
          class="grid grid-cols-[max-content_minmax(0,1fr)] gap-2 text-xs leading-5"
        >
          <span class="shrink-0 text-muted-foreground">{{ row.label }}</span>
          <span class="truncate font-mono text-foreground/90" :title="row.value">{{ row.value }}</span>
        </div>
      </div>

      <!--
        The pointer: a small square turned 45°, wearing the two borders that face out of the bubble
        and the bubble's own colour, so what shows is a tip. Drawn by hand rather than with the
        library's arrow, which is an open SVG path — filling it would draw the edge that sits on the
        bubble's own border as well, and the reference's is this square.
      -->
      <span
        class="absolute top-1/2 -left-1.25 size-2.5 -translate-y-1/2 rotate-45 rounded-[2px] border-b border-l border-border bg-card"
        aria-hidden="true"
      ></span>
    </HoverCardContent>
  </HoverCardPortal>
</template>
