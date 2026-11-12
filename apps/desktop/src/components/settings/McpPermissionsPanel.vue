<script setup lang="ts">
/**
 * The permissions tab: the default tier, and the exceptions that groups and sessions make to it.
 *
 * Four rules shape it:
 *
 * 1. **The default is a card, the exceptions are a tree.** The default applies to every saved
 *    session that says nothing of its own, so it is one choice drawn three ways. An exception is a
 *    fact about one row of the session library, so it is drawn on that row.
 * 2. **A session's own tier wins, then its group's, then the default.** The line beside each row
 *    says both the tier that is in force and which of the three chose it — a number without its
 *    source is a number somebody cannot debug.
 * 3. **Visibility is an allowlist only after the first hide.** `null` means every saved session;
 *    the first hide materialises the ids that exist at that moment, and a session saved afterwards
 *    starts hidden until it is ticked. The page says so, because that is a rule people do not guess.
 * 4. **The default is applied at once; the exceptions are a draft.** A card writes the same
 *    `mcpScope` the access tab's dropdown writes, so the two controls cannot disagree. The ticks and
 *    the two override maps are written only by Apply, which also re-reads the listener so what is
 *    drawn afterwards is what the host now holds.
 */
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";

import Select from "@/components/ui/Select.vue";
import Switch from "@/components/ui/Switch.vue";
import { MCP_SCOPE_HINT_KEYS, MCP_SCOPE_LABEL_KEYS } from "@/lib/mcp/labels";
import {
  effectiveScopeFor,
  groupScopeFor,
  hideSessions,
  isSessionVisible,
  showSessions,
  withGroupScope,
  withSessionScope,
  type EffectiveScope,
} from "@/lib/mcp/permissions";
import { TABLE, rowClass } from "@/lib/table";
import { useMcpStore } from "@/stores/mcp";
import { usePreferencesStore } from "@/stores/preferences";
import { useSessionsStore } from "@/stores/sessions";
import { DEFAULT_MCP_SCOPE, type McpAccessPolicy, type McpScope } from "@/types/app";
import { isMcpScope } from "@/types/mcp";
import type { Session, SessionGroup } from "@/types/session";

const { t } = useI18n();
const mcp = useMcpStore();
const preferences = usePreferencesStore();
const library = useSessionsStore();

/** The three tiers, in the order the cards and the table draw them. */
const SCOPES: readonly McpScope[] = ["readOnly", "safeWrite", "highRisk"];

/** The draft: the two override maps and the visibility list. The default tier is a preference. */
const visibleSessionIds = ref<readonly string[] | null>(null);
const sessionScopes = ref<Readonly<Record<string, McpScope>>>({});
const groupScopes = ref<Readonly<Record<string, McpScope>>>({});

/** What the last apply did, said under the button. */
const notice = ref<"" | "applied" | "failed">("");
const applying = ref(false);
/** What the tree is narrowed to. */
const query = ref("");
const onlyOverrides = ref(false);

/**
 * The default tier as the store holds it, which is applied the moment a card is pressed.
 *
 * The one value on this page that is not a draft: it already has a control on the access tab, and
 * two draft copies of one setting is how the two tabs would come to disagree. Everything else here
 * — the ticks and the two override maps — is a draft until Apply.
 */
const currentScope = computed<McpScope>(() => preferences.mcpScope ?? DEFAULT_MCP_SCOPE);

/** The three cards: a name and the sentence that says what choosing it allows. */
const scopeCards = computed(() =>
  SCOPES.map((scope) => ({
    value: scope,
    label: t(MCP_SCOPE_LABEL_KEYS[scope]),
    hint: t(MCP_SCOPE_HINT_KEYS[scope]),
  })),
);

/** The dropdown's answers: no override, or one of the three tiers. */
const scopeOptions = computed(() => [
  { value: "", label: t("mcp.permissions.inherit") },
  ...SCOPES.map((scope) => ({ value: scope, label: t(MCP_SCOPE_LABEL_KEYS[scope]) })),
]);

/** The draft as the policy type, for the pure functions to read. */
const draft = computed<McpAccessPolicy>(() => ({
  visibleSessionIds: visibleSessionIds.value,
  sessionScopes: sessionScopes.value,
  groupScopes: groupScopes.value,
}));

const allSessionIds = computed<readonly string[]>(() => library.sessions.map((session) => session.id));

/**
 * Takes the draft from the preferences store, which is what the file said when it was read.
 *
 * Also run after a successful apply, because the host is the authority on what was written: a value
 * it normalized on the way in has to come back out, not stay as the page drew it.
 */
function adoptPolicy(): void {
  const policy = preferences.mcpAccessPolicy;
  visibleSessionIds.value = policy.visibleSessionIds === null ? null : [...policy.visibleSessionIds];
  sessionScopes.value = { ...policy.sessionScopes };
  groupScopes.value = { ...policy.groupScopes };
}

onMounted(() => {
  adoptPolicy();
  if (library.status !== "ready") {
    void library.load();
  }
});

// -- The tree ----------------------------------------------------------------

/** Groups under a parent, pinned ones first, then the arrangement somebody dragged them into. */
function groupsUnder(parentId: string | undefined): SessionGroup[] {
  return library.groups
    .filter((group) => (group.parentId ?? undefined) === parentId)
    .slice()
    .sort(
      (left, right) =>
        Number(right.pinned) - Number(left.pinned) || left.order - right.order || left.name.localeCompare(right.name),
    );
}

/** The sessions sitting directly in a group, or at the root when no group is named. */
function sessionsIn(groupId: string | undefined): Session[] {
  return library.sessions
    .filter((session) => (session.groupId ?? undefined) === groupId)
    .slice()
    .sort((left, right) => left.name.localeCompare(right.name));
}

/** One drawn row: a heading, or a session. Both carry their depth so the tree indents the same way. */
interface PermissionRow {
  readonly kind: "group" | "session";
  readonly key: string;
  readonly depth: number;
  /** The heading a group row draws; `null` on a session row. */
  readonly group: SessionGroup | null;
  /** The session a session row draws; `null` on a group row. */
  readonly session: Session | null;
  /** Every session under a group row, subgroups included: what a heading's tick covers. */
  readonly memberIds: readonly string[];
}

/** A group and everything under it, before the filters decide what is drawn. */
interface GroupNode {
  readonly group: SessionGroup;
  readonly depth: number;
  readonly children: GroupNode[];
  readonly sessions: Session[];
}

function buildTree(): GroupNode[] {
  const node = (group: SessionGroup, depth: number): GroupNode => ({
    group,
    depth,
    children: groupsUnder(group.id).map((child) => node(child, depth + 1)),
    sessions: sessionsIn(group.id),
  });
  return groupsUnder(undefined).map((group) => node(group, 0));
}

const tree = computed<GroupNode[]>(buildTree);

function hasSessionOverride(session: Session): boolean {
  return sessionScopes.value[session.id] !== undefined || !isSessionVisible(draft.value, session.id);
}

function hasGroupOverride(groupId: string): boolean {
  return groupScopes.value[groupId] !== undefined;
}

function groupName(groupId: string): string {
  return library.groups.find((group) => group.id === groupId)?.name ?? groupId;
}

/**
 * Applies the two filters to one group, keeping the headings of whatever survives.
 *
 * A group stays when its own name matches (and, with "only exceptions" on, when it has one), when a
 * child group survives, or when a session under it survives. A heading with nothing under it and no
 * matching name is what a filter is for: it goes.
 */
function filterNode(node: GroupNode): PermissionRow[] {
  const needle = query.value.trim().toLowerCase();
  const nameMatches = needle === "" || node.group.name.toLowerCase().includes(needle);

  const childRows = node.children.flatMap(filterNode);
  const sessionRows = node.sessions
    .filter((session) => {
      const ownMatch =
        needle === "" ||
        session.name.toLowerCase().includes(needle) ||
        (session.groupId !== undefined && groupName(session.groupId).toLowerCase().includes(needle));
      if (!ownMatch) {
        return false;
      }
      return !onlyOverrides.value || hasSessionOverride(session);
    })
    .map<PermissionRow>((session) => ({
      kind: "session",
      key: `session:${session.id}`,
      depth: node.depth + 1,
      group: null,
      session,
      memberIds: [],
    }));

  const own = nameMatches && (!onlyOverrides.value || hasGroupOverride(node.group.id));
  if (!own && childRows.length === 0 && sessionRows.length === 0) {
    return [];
  }

  return [
    {
      kind: "group",
      key: `group:${node.group.id}`,
      depth: node.depth,
      group: node.group,
      session: null,
      memberIds: library.sessionsUnder(node.group.id),
    },
    ...childRows,
    ...sessionRows,
  ];
}

/** The rows on screen: the filtered tree, then the sessions that belong to no group. */
const rows = computed<PermissionRow[]>(() => {
  const needle = query.value.trim().toLowerCase();
  const ungrouped = sessionsIn(undefined)
    .filter((session) => {
      const ownMatch = needle === "" || session.name.toLowerCase().includes(needle);
      return ownMatch && (!onlyOverrides.value || hasSessionOverride(session));
    })
    .map<PermissionRow>((session) => ({
      kind: "session",
      key: `session:${session.id}`,
      depth: 0,
      group: null,
      session,
      memberIds: [],
    }));
  return [...tree.value.flatMap(filterNode), ...ungrouped];
});

// -- Visibility --------------------------------------------------------------

function sessionVisible(sessionId: string): boolean {
  return visibleSessionIds.value === null || visibleSessionIds.value.includes(sessionId);
}

/** How much of a group's subtree is visible: all of it, none, or somewhere in between. */
function groupVisibility(memberIds: readonly string[]): "all" | "none" | "some" {
  if (memberIds.length === 0) {
    // Nothing to tick: the box is disabled, and unchecked says that better than a vacuous tick.
    return "none";
  }
  const visible = memberIds.filter(sessionVisible).length;
  if (visible === 0) {
    return "none";
  }
  return visible === memberIds.length ? "all" : "some";
}

function setMembersVisible(memberIds: readonly string[], visible: boolean): void {
  const next = visible
    ? showSessions(draft.value, memberIds)
    : hideSessions(draft.value, memberIds, allSessionIds.value);
  visibleSessionIds.value = next.visibleSessionIds;
  notice.value = "";
}

/** Drops the allowlist back to "every session", including ones saved later. */
function allowEverySession(): void {
  visibleSessionIds.value = null;
  notice.value = "";
}

// -- Scopes ------------------------------------------------------------------

function setSessionScope(sessionId: string, value: string): void {
  const scope = isMcpScope(value) ? value : null;
  sessionScopes.value = { ...withSessionScope(draft.value, sessionId, scope).sessionScopes };
  notice.value = "";
}

function setGroupScope(groupId: string, value: string): void {
  const scope = isMcpScope(value) ? value : null;
  groupScopes.value = { ...withGroupScope(draft.value, groupId, scope).groupScopes };
  notice.value = "";
}

/**
 * The dropdown handlers take the row rather than its id.
 *
 * The template narrows `row.group`/`row.session` inside the branch, but that narrowing does not
 * cross into an arrow function, so the guard is written here where both the reader and the compiler
 * see it.
 */
function changeSessionScope(row: PermissionRow, value: string): void {
  if (row.session) {
    setSessionScope(row.session.id, value);
  }
}

function changeGroupScope(row: PermissionRow, value: string): void {
  if (row.group) {
    setGroupScope(row.group.id, value);
  }
}

function sessionEffective(session: Session): EffectiveScope {
  return effectiveScopeFor(session, draft.value, currentScope.value, library.groups);
}

function groupEffective(group: SessionGroup): EffectiveScope {
  return groupScopeFor(group.id, draft.value, currentScope.value, library.groups);
}

/** The tier in force, and which rule chose it, as one line under the dropdown. */
function effectiveLabel(effective: EffectiveScope): string {
  const scope = t(MCP_SCOPE_LABEL_KEYS[effective.scope]);
  if (effective.source === "session") {
    return `${scope} · ${t("mcp.permissions.source.session")}`;
  }
  if (effective.source === "group") {
    return `${scope} · ${t("mcp.permissions.source.group", { group: groupName(effective.groupId ?? "") })}`;
  }
  return `${scope} · ${t("mcp.permissions.source.global")}`;
}

/** A group's own tier and where it came from; a group has no session above it to inherit from. */
function groupEffectiveLabel(group: SessionGroup): string {
  const effective = groupEffective(group);
  const scope = t(MCP_SCOPE_LABEL_KEYS[effective.scope]);
  return effective.source === "group"
    ? `${scope} · ${t("mcp.permissions.source.group", { group: group.name })}`
    : `${scope} · ${t("mcp.permissions.source.global")}`;
}

/**
 * Writes the default and both override maps, then re-reads the listener.
 *
 * The catalog is deliberately not re-read here: permissions do not change which tools exist, and a
 * reload would reset an unapplied capability draft on the other tab.
 */
async function apply(): Promise<void> {
  if (applying.value) {
    return;
  }
  notice.value = "";
  applying.value = true;
  preferences.setMcpAccessPolicy({
    visibleSessionIds: visibleSessionIds.value === null ? null : [...visibleSessionIds.value],
    sessionScopes: { ...sessionScopes.value },
    groupScopes: { ...groupScopes.value },
  });
  try {
    if (!(await preferences.commit())) {
      notice.value = "failed";
      return;
    }
    adoptPolicy();
    await mcp.refresh();
    notice.value = "applied";
  } finally {
    applying.value = false;
  }
}
</script>

<template>
  <section class="space-y-3" data-mcp-permissions>
    <div>
      <h4 class="text-sm font-medium">{{ t("mcp.permissions.title") }}</h4>
      <p class="mt-0.5 text-xs leading-relaxed text-muted-foreground">{{ t("mcp.permissions.description") }}</p>
    </div>

    <!-- The default: one choice, drawn as the three tiers it can be. -->
    <div class="space-y-2 rounded-md border border-border px-3 py-3">
      <h5 class="text-xs font-medium">{{ t("mcp.permissions.defaultTitle") }}</h5>
      <div class="grid gap-2 sm:grid-cols-3">
        <button
          v-for="card in scopeCards"
          :key="card.value"
          type="button"
          :data-mcp-scope-card="card.value"
          :aria-pressed="currentScope === card.value"
          class="rounded-md border px-3 py-2 text-left transition-colors"
          :class="currentScope === card.value ? 'border-primary bg-primary/10' : 'border-border hover:bg-accent/50'"
          @click="preferences.setMcpScope(card.value)"
        >
          <span class="block text-xs font-medium">{{ card.label }}</span>
          <span class="mt-0.5 block text-[11px] leading-relaxed text-muted-foreground">{{ card.hint }}</span>
        </button>
      </div>
      <p class="text-[11px] leading-relaxed text-muted-foreground">{{ t("mcp.permissions.defaultHint") }}</p>
    </div>

    <!-- What each tier means, in one place: the cards say it too, but a table can be compared. -->
    <div class="space-y-2 rounded-md border border-border px-3 py-3">
      <h5 class="text-xs font-medium">{{ t("mcp.permissions.table.title") }}</h5>
      <table :class="TABLE.page.table" :aria-label="t('mcp.permissions.table.title')">
        <thead :class="TABLE.page.head">
          <tr :class="TABLE.page.headRow">
            <th class="text-left" :class="TABLE.page.headCell">{{ t("mcp.permissions.table.tier") }}</th>
            <th class="text-left" :class="TABLE.page.headCell">{{ t("mcp.permissions.table.allows") }}</th>
            <th class="text-left" :class="TABLE.page.headCell">{{ t("mcp.permissions.table.asks") }}</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="scope in SCOPES" :key="scope" :class="rowClass('page')">
            <td class="whitespace-nowrap font-medium" :class="TABLE.page.cell">
              {{ t(MCP_SCOPE_LABEL_KEYS[scope]) }}
            </td>
            <td :class="[TABLE.page.cell, 'text-muted-foreground']">{{ t(MCP_SCOPE_HINT_KEYS[scope]) }}</td>
            <td :class="[TABLE.page.cell, 'text-muted-foreground']">{{ t("mcp.permissions.table.asksValue") }}</td>
          </tr>
        </tbody>
      </table>
    </div>

    <!-- The exceptions: the session library, its groups, and what each row is allowed. -->
    <div class="space-y-2 rounded-md border border-border px-3 py-3">
      <div class="flex items-start justify-between gap-3">
        <div class="min-w-0">
          <h5 class="text-xs font-medium">{{ t("mcp.permissions.sessionsTitle") }}</h5>
          <p class="mt-0.5 text-[11px] leading-relaxed text-muted-foreground">
            {{ t("mcp.permissions.sessionsDescription") }}
          </p>
        </div>
        <label class="flex shrink-0 items-center gap-2 text-[11px] text-muted-foreground">
          <Switch
            data-mcp-permission-overrides
            :model-value="onlyOverrides"
            :aria-label="t('mcp.permissions.onlyOverrides')"
            @update:model-value="(value: boolean) => (onlyOverrides = value)"
          />
          {{ t("mcp.permissions.onlyOverrides") }}
        </label>
      </div>

      <input
        data-mcp-permission-search
        :value="query"
        type="search"
        autocomplete="off"
        spellcheck="false"
        class="w-full rounded-md border border-border bg-background px-3 py-1.5 text-xs focus:outline-none select-text"
        :aria-label="t('mcp.permissions.search')"
        :placeholder="t('mcp.permissions.search')"
        @input="query = ($event.target as HTMLInputElement).value"
      />

      <div
        v-if="visibleSessionIds !== null"
        data-mcp-allowlist-hint
        class="space-y-2 rounded-md border border-warning/40 bg-warning/10 px-3 py-2 text-[11px] leading-relaxed"
      >
        <p>{{ t("mcp.permissions.allowlistHint") }}</p>
        <button
          type="button"
          data-mcp-allow-all
          class="rounded-md border border-border bg-background px-2 py-1 text-[11px] hover:bg-accent"
          @click="allowEverySession"
        >
          {{ t("mcp.permissions.allowAll") }}
        </button>
      </div>

      <p v-if="library.sessions.length === 0" class="text-xs text-muted-foreground">
        {{ library.status === "loading" ? t("mcp.permissions.loading") : t("mcp.permissions.noSessions") }}
      </p>
      <p v-else-if="rows.length === 0" class="text-xs text-muted-foreground">{{ t("mcp.permissions.empty") }}</p>

      <ul v-else class="space-y-1">
        <li
          v-for="row in rows"
          :key="row.key"
          data-mcp-permission-row
          :data-mcp-permission-kind="row.kind"
          :data-mcp-group-id="row.group?.id"
          :data-mcp-session-id="row.session?.id"
          :data-mcp-permission-depth="row.depth"
          class="rounded-md border border-border px-2.5 py-1.5"
          :class="row.kind === 'group' ? 'bg-muted/40' : 'bg-muted/20'"
          :style="{ marginLeft: `${row.depth * 16}px` }"
        >
          <!-- A group: its tick covers every session under it, subgroups included. -->
          <div v-if="row.kind === 'group' && row.group" class="flex flex-wrap items-center gap-x-2 gap-y-1">
            <input
              data-mcp-visibility
              type="checkbox"
              class="size-3.5 accent-primary"
              :checked="groupVisibility(row.memberIds) === 'all'"
              :indeterminate.prop="groupVisibility(row.memberIds) === 'some'"
              :disabled="row.memberIds.length === 0"
              :aria-label="row.group.name"
              @change="setMembersVisible(row.memberIds, ($event.target as HTMLInputElement).checked)"
            />
            <span class="text-xs font-medium">{{ row.group.name }}</span>
            <span class="text-[11px] text-muted-foreground">
              {{ t("mcp.permissions.groupCount", { count: String(row.memberIds.length) }) }}
            </span>

            <div class="ml-auto flex items-center gap-2">
              <span
                data-mcp-effective
                :data-mcp-effective-source="groupEffective(row.group).source"
                class="text-[11px] text-muted-foreground"
              >
                {{ groupEffectiveLabel(row.group) }}
              </span>
              <Select
                :data-mcp-group-scope="row.group.id"
                :model-value="groupScopes[row.group.id] ?? ''"
                :options="scopeOptions"
                size="compact"
                :aria-label="t('mcp.permissions.groupScope')"
                @update:model-value="(value: string) => changeGroupScope(row, value)"
              />
            </div>
          </div>

          <!-- A session: its own tick, its own tier, and the tier that is actually in force. -->
          <div v-else-if="row.session" class="flex flex-wrap items-center gap-x-2 gap-y-1">
            <input
              data-mcp-visibility
              type="checkbox"
              class="size-3.5 accent-primary"
              :checked="sessionVisible(row.session.id)"
              :aria-label="row.session.name"
              @change="setMembersVisible([row.session.id], ($event.target as HTMLInputElement).checked)"
            />
            <span class="text-xs" :class="sessionVisible(row.session.id) ? undefined : 'text-muted-foreground'">
              {{ row.session.name }}
            </span>
            <span
              v-if="!sessionVisible(row.session.id)"
              data-mcp-hidden
              class="rounded border border-warning/40 px-1 py-px text-[11px] text-warning"
            >
              {{ t("mcp.permissions.hidden") }}
            </span>

            <div class="ml-auto flex flex-wrap items-center gap-2">
              <span
                data-mcp-effective
                :data-mcp-effective-source="sessionEffective(row.session).source"
                :data-mcp-effective-scope="sessionEffective(row.session).scope"
                class="text-[11px] text-muted-foreground"
              >
                {{ effectiveLabel(sessionEffective(row.session)) }}
              </span>
              <Select
                :data-mcp-session-scope="row.session.id"
                :model-value="sessionScopes[row.session.id] ?? ''"
                :options="scopeOptions"
                size="compact"
                :aria-label="t('mcp.permissions.sessionScope')"
                @update:model-value="(value: string) => changeSessionScope(row, value)"
              />
            </div>
          </div>
        </li>
      </ul>
    </div>

    <div class="flex flex-wrap items-center gap-2">
      <button
        type="button"
        data-mcp-apply="permissions"
        class="rounded-md bg-primary px-3 py-1.5 text-xs text-primary-foreground transition-colors hover:bg-primary/90 disabled:pointer-events-none disabled:opacity-50"
        :disabled="applying"
        @click="apply()"
      >
        {{ t("mcp.permissions.apply") }}
      </button>
      <span v-if="notice === 'applied'" data-mcp-applied="permissions" class="text-xs text-success">{{
        t("mcp.permissions.applied")
      }}</span>
      <span v-else-if="notice === 'failed'" class="text-xs text-destructive">
        {{ t("mcp.permissions.failed") }}
      </span>
    </div>
  </section>
</template>
