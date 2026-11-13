<script setup lang="ts">
/**
 * What a ready external server offers, in two halves: resources and prompts.
 *
 * The two halves are deliberately different things rather than two flavours of the same list:
 *
 * 1. **A resource becomes an attachment.** It is read first, drawn exactly as it came back, and only
 *    then offered for attaching — a reader who has not seen the text cannot have decided to send it.
 *    Only text is attachable; a binary resource is still previewed (an image is drawn, anything else
 *    is named as binary), because "what is in there" is a question worth answering even when the
 *    answer cannot travel to the model.
 * 2. **A prompt becomes text in the composer.** Rendering fills one input per argument the server
 *    declared, the preview is the exact string that will be inserted, and Insert only appends it —
 *    nothing is sent. The person edits the draft and decides.
 *
 * Neither half asks for approval and neither writes an audit line: reading a resource or rendering a
 * prompt changes nothing on the far side. That is why this dialog can call the two commands directly
 * rather than going through the tool registry.
 *
 * A server that declares neither capability is simply not grouped here; an empty answer is a calm
 * sentence, not a failure — most installations have no external server at all.
 */
import { LoaderCircle, X } from "@lucide/vue";
import { DialogContent, DialogDescription, DialogOverlay, DialogPortal, DialogRoot, DialogTitle } from "reka-ui";
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import { type DescribedError, describeShellError } from "@/i18n/errors";
import { backend } from "@/lib/backend/api";
import { asShellError } from "@/lib/backend/errors";
import { useAssistantStore } from "@/stores/assistant";
import type { McpPrompt, McpPromptText, McpResource, McpResourceRead, McpServerContent } from "@/types/mcp";

const props = defineProps<{ open: boolean }>();

const emit = defineEmits<{
  "update:open": [value: boolean];
  /** The rendered text a prompt produced, to be appended to the composer by the panel. */
  insert: [text: string];
}>();

const { t } = useI18n();
const assistant = useAssistantStore();

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

/** Which half of the picker is on screen. */
const tab = ref<"resources" | "prompts">("resources");

/** The catalog as the host answered it. Refetched on every open — servers connect and drop between. */
const catalog = ref<McpServerContent[]>([]);
const loading = ref(false);
/** The catalog has been asked for at least once, so an empty list is an answer rather than "not yet". */
const loaded = ref(false);
const catalogProblem = ref<DescribedError | null>(null);

/** One resource read, by `serverId` and URI. */
interface ReadState {
  status: "reading" | "ready" | "failed";
  read?: McpResourceRead;
  problem?: DescribedError;
}

/** One prompt's draft arguments and the render it produced. */
interface PromptState {
  args: Record<string, string>;
  status: "idle" | "rendering" | "ready" | "inserted" | "failed";
  text?: McpPromptText;
  problem?: DescribedError;
}

const reads = ref<Record<string, ReadState>>({});
const promptStates = ref<Record<string, PromptState>>({});
/** The store refused one more attachment: said once, under the button that tried. */
const attachFull = ref(false);

function readKey(serverId: string, uri: string): string {
  return `${serverId}\n${uri}`;
}

function promptKey(serverId: string, name: string): string {
  return `${serverId}:${name}`;
}

/** The servers with something to offer in the half on screen. A capability nobody declared is absent. */
const groups = computed(() =>
  tab.value === "resources"
    ? catalog.value.filter((server) => server.resources.length > 0)
    : catalog.value.filter((server) => server.prompts.length > 0),
);

/** Whether the half on screen is empty because nothing at all was offered, or because this half is. */
const emptyMessage = computed(() => (catalog.value.length === 0 ? t("ai.content.empty") : t("ai.content.emptyTab")));

async function loadCatalog(): Promise<void> {
  loading.value = true;
  catalogProblem.value = null;
  try {
    catalog.value = await backend.mcp.contentCatalog();
    loaded.value = true;
    // One draft per prompt, made now: a box that appears only for prompts somebody happened to touch
    // would lose what was typed the first time the tab was left and come back.
    const drafts: Record<string, PromptState> = {};
    for (const server of catalog.value) {
      for (const prompt of server.prompts) {
        drafts[promptKey(server.serverId, prompt.name)] = {
          args: Object.fromEntries(prompt.arguments.map((argument) => [argument.name, ""])),
          status: "idle",
        };
      }
    }
    promptStates.value = drafts;
  } catch (error) {
    // The list is the dialog's own failure: it does not touch the chat, the tools or the servers.
    catalogProblem.value = describeShellError(translate, asShellError(error, "mcp_content_catalog"));
  } finally {
    loaded.value = true;
    loading.value = false;
  }
}

watch(
  () => props.open,
  (open) => {
    if (!open) {
      return;
    }
    tab.value = "resources";
    reads.value = {};
    promptStates.value = {};
    catalog.value = [];
    loaded.value = false;
    attachFull.value = false;
    catalogProblem.value = null;
    void loadCatalog();
  },
  { immediate: true },
);

/** The read of one resource, when it has been asked for. */
function readState(server: McpServerContent, resource: McpResource): ReadState | undefined {
  return reads.value[readKey(server.serverId, resource.uri)];
}

async function readResource(server: McpServerContent, resource: McpResource): Promise<void> {
  const key = readKey(server.serverId, resource.uri);
  attachFull.value = false;
  reads.value = { ...reads.value, [key]: { status: "reading" } };
  try {
    const read = await backend.mcp.readResource(server.serverId, resource.uri);
    reads.value = { ...reads.value, [key]: { status: "ready", read } };
  } catch (error) {
    reads.value = {
      ...reads.value,
      [key]: { status: "failed", problem: describeShellError(translate, asShellError(error, "mcp_resource_read")) },
    };
  }
}

/**
 * The text an attachment would carry.
 *
 * A read may answer with several pieces; the context has room for one text field, so they are joined
 * with a blank line. Nothing is re-wrapped or trimmed — the pieces are the server's own bytes.
 */
function attachableText(read: McpResourceRead): string {
  return read.contents
    .filter((content) => content.text !== undefined)
    .map((content) => content.text ?? "")
    .join("\n\n");
}

/** Only text travels; a read made entirely of blobs has nothing an assistant could read. */
function canAttach(state: ReadState | undefined): boolean {
  return (
    state?.read !== undefined &&
    state.read.contents.some((content) => content.text !== undefined && content.text !== "")
  );
}

/** Whether this resource is already in the next question's attachments. */
function isAttached(server: McpServerContent, resource: McpResource): boolean {
  return assistant.attachments.some(
    (attachment) => attachment.server === server.serverId && attachment.uri === resource.uri,
  );
}

function attach(server: McpServerContent, resource: McpResource): void {
  const state = readState(server, resource);
  if (state?.read === undefined || !canAttach(state)) {
    return;
  }
  const accepted = assistant.attachResource({
    server: server.serverId,
    uri: resource.uri,
    name: resource.title ?? resource.name,
    mimeType: resource.mimeType ?? state.read.contents[0]?.mimeType,
    text: attachableText(state.read),
  });
  attachFull.value = !accepted;
}

/** The mime type of one returned piece, as the preview needs it. */
function pieceMime(content: { mimeType?: string }, resource: McpResource): string {
  return content.mimeType ?? resource.mimeType ?? "";
}

/** A base64 blob drawn as an image, straight from the read — nothing is re-encoded or written down. */
function dataUrl(mimeType: string, blob: string): string {
  return `data:${mimeType};base64,${blob}`;
}

function promptState(server: McpServerContent, prompt: McpPrompt): PromptState | undefined {
  return promptStates.value[promptKey(server.serverId, prompt.name)];
}

/** Types into one argument, keeping the draft and clearing the last render's complaint. */
function setPromptArgument(server: McpServerContent, prompt: McpPrompt, name: string, event: Event): void {
  const key = promptKey(server.serverId, prompt.name);
  const state = promptStates.value[key];
  if (state === undefined) {
    return;
  }
  const value = (event.target as HTMLInputElement).value;
  promptStates.value = {
    ...promptStates.value,
    [key]: { ...state, args: { ...state.args, [name]: value }, problem: undefined },
  };
}

/** The text the prompt preview draws, and the text Insert appends — one string, computed once. */
function promptPreview(server: McpServerContent, prompt: McpPrompt): string {
  const text = promptState(server, prompt)?.text;
  return text === undefined ? "" : renderedText(text);
}

async function renderPrompt(server: McpServerContent, prompt: McpPrompt): Promise<void> {
  const key = promptKey(server.serverId, prompt.name);
  const state = promptStates.value[key];
  if (state === undefined || state.status === "rendering") {
    return;
  }
  const missing = prompt.arguments
    .filter((argument) => argument.required && (state.args[argument.name] ?? "").trim() === "")
    .map((argument) => argument.name);
  if (missing.length > 0) {
    promptStates.value = {
      ...promptStates.value,
      [key]: {
        ...state,
        status: "failed",
        text: undefined,
        problem: { message: t("ai.content.prompt.requiredMissing", { names: missing.join(", ") }), code: "" },
      },
    };
    return;
  }

  promptStates.value = { ...promptStates.value, [key]: { ...state, status: "rendering", problem: undefined } };
  try {
    const text = await backend.mcp.getPrompt(server.serverId, prompt.name, state.args);
    // Re-read the draft: typing may have gone on while the server was answering, and writing the
    // state captured before the await would put the older arguments back on screen.
    const current = promptStates.value[key] ?? state;
    promptStates.value = { ...promptStates.value, [key]: { ...current, status: "ready", text, problem: undefined } };
  } catch (error) {
    const current = promptStates.value[key] ?? state;
    promptStates.value = {
      ...promptStates.value,
      [key]: {
        ...current,
        status: "failed",
        text: undefined,
        problem: describeShellError(translate, asShellError(error, "mcp_prompt_get")),
      },
    };
  }
}

/**
 * The rendered prompt as the text that will be inserted: one line per message, the role first, a
 * blank line between messages. The same string is previewed, so what is read is what arrives in the
 * composer.
 */
function renderedText(text: McpPromptText): string {
  return text.messages.map((message) => `${message.role}: ${message.text}`).join("\n\n");
}

function insertPrompt(server: McpServerContent, prompt: McpPrompt): void {
  const key = promptKey(server.serverId, prompt.name);
  const state = promptStates.value[key];
  if (state?.text === undefined) {
    return;
  }
  emit("insert", renderedText(state.text));
  // Left open on purpose: inserting one prompt is often followed by attaching a resource, and a
  // dialog that closed itself would have to be opened again for the second half of the same task.
  promptStates.value = { ...promptStates.value, [key]: { ...state, status: "inserted" } };
}
</script>

<template>
  <DialogRoot :open="props.open" @update:open="(value: boolean) => emit('update:open', value)">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-50 bg-overlay" />
      <DialogContent
        data-mcp-content-dialog
        class="fixed top-1/2 left-1/2 z-50 flex max-h-[min(82vh,46rem)] w-[min(46rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 flex-col rounded-lg border border-border bg-card shadow-xl select-none focus:outline-none"
      >
        <header class="flex shrink-0 items-start gap-2 border-b border-border px-4 py-2.5">
          <div class="min-w-0 flex-1">
            <DialogTitle class="text-sm font-medium">{{ t("ai.content.title") }}</DialogTitle>
            <DialogDescription class="mt-0.5 text-[10px] leading-relaxed text-muted-foreground">
              {{ t("ai.content.description") }}
            </DialogDescription>
          </div>
          <button
            type="button"
            class="grid size-5 shrink-0 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            :title="t('common.action.close')"
            :aria-label="t('common.action.close')"
            @click="emit('update:open', false)"
          >
            <X :size="12" />
          </button>
        </header>

        <div class="flex shrink-0 items-center gap-1 border-b border-border px-4">
          <button
            type="button"
            data-mcp-content-tab="resources"
            class="border-b-2 px-2 py-1.5 text-[11px] transition-colors"
            :class="
              tab === 'resources'
                ? 'border-primary text-foreground'
                : 'border-transparent text-muted-foreground hover:text-foreground'
            "
            @click="tab = 'resources'"
          >
            {{ t("ai.content.tabResources") }}
          </button>
          <button
            type="button"
            data-mcp-content-tab="prompts"
            class="border-b-2 px-2 py-1.5 text-[11px] transition-colors"
            :class="
              tab === 'prompts'
                ? 'border-primary text-foreground'
                : 'border-transparent text-muted-foreground hover:text-foreground'
            "
            @click="tab = 'prompts'"
          >
            {{ t("ai.content.tabPrompts") }}
          </button>
        </div>

        <div class="min-h-0 flex-1 space-y-3 overflow-y-auto px-4 py-3 select-text">
          <p v-if="loading" data-mcp-content-loading class="text-[11px] text-muted-foreground">
            {{ t("ai.content.loading") }}
          </p>

          <p v-else-if="catalogProblem" data-mcp-content-error class="text-[11px] text-destructive">
            <span>{{ catalogProblem.message }}</span>
            <span v-if="catalogProblem.code !== ''" class="mt-0.5 block font-mono text-[10px] text-muted-foreground">
              {{ catalogProblem.code }}
            </span>
          </p>

          <p
            v-else-if="loaded && groups.length === 0"
            data-mcp-content-empty
            class="text-[11px] leading-relaxed text-muted-foreground"
          >
            {{ emptyMessage }}
          </p>

          <!-- Resources: one group per server, one row per resource, one read and attach per row. -->
          <template v-if="tab === 'resources'">
            <section v-for="server in groups" :key="server.serverId" class="space-y-1.5">
              <h4 class="flex items-baseline gap-1.5">
                <span class="text-[11px] font-medium">{{ server.name }}</span>
                <span class="font-mono text-[10px] text-muted-foreground">{{ server.serverId }}</span>
              </h4>

              <div
                v-for="resource in server.resources"
                :key="resource.uri"
                :data-mcp-resource="resource.uri"
                :data-mcp-resource-server="server.serverId"
                class="space-y-1 rounded-md border border-border bg-background/40 px-2.5 py-1.5"
              >
                <div class="flex items-start gap-2">
                  <div class="min-w-0 flex-1">
                    <p class="truncate text-[11px]">{{ resource.title ?? resource.name }}</p>
                    <p
                      v-if="resource.title !== undefined && resource.title !== resource.name"
                      class="truncate text-[10px] text-muted-foreground"
                    >
                      {{ resource.name }}
                    </p>
                    <p class="font-mono text-[10px] break-all text-muted-foreground">{{ resource.uri }}</p>
                    <p class="text-[10px] text-muted-foreground">
                      <span v-if="resource.mimeType">{{ resource.mimeType }}</span>
                      <span v-if="resource.mimeType && resource.description"> · </span>
                      <span v-if="resource.description" class="break-words">{{ resource.description }}</span>
                    </p>
                  </div>

                  <button
                    type="button"
                    data-mcp-resource-read
                    class="shrink-0 rounded-md border border-border px-2 py-0.5 text-[10px] transition-colors hover:bg-accent disabled:pointer-events-none disabled:opacity-50"
                    :disabled="readState(server, resource)?.status === 'reading'"
                    @click="readResource(server, resource)"
                  >
                    <LoaderCircle
                      v-if="readState(server, resource)?.status === 'reading'"
                      :size="10"
                      class="mr-1 inline animate-spin"
                    />
                    {{ t("ai.content.resource.read") }}
                  </button>
                </div>

                <p v-if="readState(server, resource)?.problem" class="text-[10px] text-destructive">
                  <span>{{ readState(server, resource)?.problem?.message }}</span>
                  <span class="ml-1 font-mono text-muted-foreground">{{
                    readState(server, resource)?.problem?.code
                  }}</span>
                </p>

                <div
                  v-if="readState(server, resource)?.read"
                  data-mcp-content-preview
                  class="space-y-1 rounded-md border border-border bg-muted/30 px-2 py-1.5"
                >
                  <div class="flex items-center gap-2">
                    <span class="min-w-0 flex-1 text-[10px] text-muted-foreground">
                      {{ t("ai.content.resource.preview") }}
                    </span>
                    <span v-if="readState(server, resource)?.read?.truncated" class="shrink-0 text-[10px] text-warning">
                      {{ t("ai.content.resource.truncated") }}
                    </span>
                    <button
                      type="button"
                      data-mcp-resource-attach
                      class="shrink-0 rounded-md border border-border px-2 py-0.5 text-[10px] transition-colors hover:bg-accent disabled:pointer-events-none disabled:opacity-50"
                      :disabled="!canAttach(readState(server, resource))"
                      :title="
                        canAttach(readState(server, resource))
                          ? t('ai.content.resource.attachHint')
                          : t('ai.content.resource.noText')
                      "
                      @click="attach(server, resource)"
                    >
                      {{
                        isAttached(server, resource)
                          ? t("ai.content.resource.attached")
                          : t("ai.content.resource.attach")
                      }}
                    </button>
                  </div>

                  <template
                    v-for="(content, index) in readState(server, resource)?.read?.contents ?? []"
                    :key="`${content.uri}:${index}`"
                  >
                    <pre
                      v-if="content.text !== undefined"
                      class="max-h-40 overflow-auto rounded-sm border border-border bg-background/60 px-2 py-1 font-mono text-[10px] leading-relaxed whitespace-pre-wrap break-words"
                      >{{ content.text }}</pre
                    >
                    <img
                      v-else-if="content.blob !== undefined && pieceMime(content, resource).startsWith('image/')"
                      :src="dataUrl(pieceMime(content, resource), content.blob)"
                      :alt="t('ai.content.resource.image')"
                      class="max-h-40 rounded-sm border border-border"
                    />
                    <p v-else-if="content.blob !== undefined" class="text-[10px] text-muted-foreground">
                      {{
                        t("ai.content.resource.binary", {
                          mime:
                            pieceMime(content, resource) === ""
                              ? t("ai.content.resource.unknownType")
                              : pieceMime(content, resource),
                        })
                      }}
                    </p>
                  </template>
                </div>
              </div>
            </section>

            <p v-if="attachFull" data-mcp-content-full class="text-[10px] text-destructive">
              {{ t("ai.content.resource.full") }}
            </p>
          </template>

          <!-- Prompts: one group per server, arguments, a render, and the text Insert will append. -->
          <template v-else>
            <section v-for="server in groups" :key="server.serverId" class="space-y-1.5">
              <h4 class="flex items-baseline gap-1.5">
                <span class="text-[11px] font-medium">{{ server.name }}</span>
                <span class="font-mono text-[10px] text-muted-foreground">{{ server.serverId }}</span>
              </h4>

              <div
                v-for="prompt in server.prompts"
                :key="prompt.name"
                :data-mcp-prompt="`${server.serverId}:${prompt.name}`"
                class="space-y-1.5 rounded-md border border-border bg-background/40 px-2.5 py-1.5"
              >
                <div class="min-w-0">
                  <p class="truncate text-[11px]">{{ prompt.title ?? prompt.name }}</p>
                  <p v-if="prompt.title !== undefined" class="font-mono text-[10px] break-all text-muted-foreground">
                    {{ prompt.name }}
                  </p>
                  <p v-if="prompt.description" class="text-[10px] leading-relaxed break-words text-muted-foreground">
                    {{ prompt.description }}
                  </p>
                </div>

                <div v-if="prompt.arguments.length > 0" class="space-y-1">
                  <label v-for="argument in prompt.arguments" :key="argument.name" class="flex flex-col gap-0.5">
                    <span class="text-[10px] text-muted-foreground">
                      {{ argument.title ?? argument.name }}
                      <span v-if="argument.required" class="text-destructive">*</span>
                    </span>
                    <input
                      :value="promptState(server, prompt)?.args[argument.name] ?? ''"
                      :data-mcp-prompt-argument="argument.name"
                      type="text"
                      spellcheck="false"
                      :required="argument.required"
                      :aria-required="argument.required"
                      :aria-label="argument.title ?? argument.name"
                      class="rounded-md border border-border bg-background px-2 py-0.5 font-mono text-[11px] focus:outline-none"
                      :placeholder="argument.description ?? argument.name"
                      @input="setPromptArgument(server, prompt, argument.name, $event)"
                    />
                  </label>
                </div>

                <div class="flex items-center gap-2">
                  <button
                    type="button"
                    data-mcp-prompt-render
                    class="shrink-0 rounded-md border border-border px-2 py-0.5 text-[10px] transition-colors hover:bg-accent disabled:pointer-events-none disabled:opacity-50"
                    :disabled="promptState(server, prompt)?.status === 'rendering'"
                    @click="renderPrompt(server, prompt)"
                  >
                    <LoaderCircle
                      v-if="promptState(server, prompt)?.status === 'rendering'"
                      :size="10"
                      class="mr-1 inline animate-spin"
                    />
                    {{ t("ai.content.prompt.render") }}
                  </button>
                  <button
                    v-if="promptState(server, prompt)?.text"
                    type="button"
                    data-mcp-prompt-insert
                    class="shrink-0 rounded-md bg-primary px-2 py-0.5 text-[10px] text-primary-foreground transition-colors hover:bg-primary/90"
                    @click="insertPrompt(server, prompt)"
                  >
                    {{ t("ai.content.prompt.insert") }}
                  </button>
                  <span
                    v-if="promptState(server, prompt)?.status === 'inserted'"
                    class="min-w-0 flex-1 truncate text-[10px] text-muted-foreground"
                  >
                    {{ t("ai.content.prompt.inserted") }}
                  </span>
                </div>

                <p v-if="promptState(server, prompt)?.problem" class="text-[10px] text-destructive">
                  <span>{{ promptState(server, prompt)?.problem?.message }}</span>
                  <span
                    v-if="promptState(server, prompt)?.problem?.code"
                    class="ml-1 font-mono text-muted-foreground"
                    >{{ promptState(server, prompt)?.problem?.code }}</span
                  >
                </p>

                <div v-if="promptState(server, prompt)?.text" class="space-y-1">
                  <p class="text-[10px] text-muted-foreground">
                    {{ t("ai.content.prompt.preview") }}
                    <span v-if="promptState(server, prompt)?.text?.truncated" class="ml-1 text-warning">{{
                      t("ai.content.prompt.truncated")
                    }}</span>
                  </p>
                  <pre
                    data-mcp-content-preview
                    data-mcp-prompt-preview
                    class="max-h-40 overflow-auto rounded-sm border border-border bg-background/60 px-2 py-1 font-mono text-[10px] leading-relaxed whitespace-pre-wrap break-words"
                    >{{ promptPreview(server, prompt) }}</pre
                  >
                </div>
              </div>
            </section>
          </template>
        </div>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
