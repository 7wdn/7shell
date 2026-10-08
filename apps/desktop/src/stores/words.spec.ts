import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({
  completionWords: vi.fn(),
  completionWordSave: vi.fn(),
  completionWordDelete: vi.fn(),
}));

import { completionWordDelete, completionWordSave, completionWords } from "@/lib/backend/tauri";
import { useWordsStore } from "@/stores/words";
import type { CompletionWord } from "@/types/snippet";

const listMock = vi.mocked(completionWords);
const saveMock = vi.mocked(completionWordSave);
const deleteMock = vi.mocked(completionWordDelete);

function word(command: string, text: string, note = ""): CompletionWord {
  return { id: `word-${command}-${text}`, command, word: text, note };
}

describe("the words store", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it("reads what is stored", async () => {
    listMock.mockResolvedValue([word("kubectl", "apply")]);
    const words = useWordsStore();

    await words.load();

    expect(words.words).toHaveLength(1);
    expect(words.loaded, "what has been read is not read again for every terminal").toBe(true);
  });

  it("writes a word and reads the list back, so the page shows what was stored", async () => {
    listMock.mockResolvedValue([word("kubectl", "apply")]);
    saveMock.mockResolvedValue(word("kubectl", "apply"));
    const words = useWordsStore();

    const written = await words.save({ id: "", command: " kubectl ", word: " apply ", note: "  " });

    expect(written).toBe(true);
    expect(saveMock).toHaveBeenCalledWith({ id: "", command: "kubectl", word: "apply", note: "" });
    expect(listMock, "the list is read again, so what is on screen is what was written").toHaveBeenCalled();
    expect(words.words).toHaveLength(1);
  });

  it("will not write half a pair", async () => {
    const words = useWordsStore();

    expect(await words.save({ id: "", command: "", word: "apply", note: "" })).toBe(false);
    expect(await words.save({ id: "", command: "kubectl", word: "   ", note: "" })).toBe(false);
    expect(saveMock).not.toHaveBeenCalled();
  });

  it("says so rather than pretending when the write did not land", async () => {
    saveMock.mockRejectedValue(new Error("no vault"));
    const words = useWordsStore();

    const written = await words.save({ id: "", command: "kubectl", word: "apply", note: "" });

    expect(written, "the dialog stays open on the word that did not land").toBe(false);
    expect(words.failure).not.toBeNull();
  });

  it("removes one by its id", async () => {
    const stored = word("kubectl", "apply");
    listMock.mockResolvedValueOnce([stored]).mockResolvedValue([]);
    deleteMock.mockResolvedValue();
    const words = useWordsStore();

    await words.load();
    await words.remove(stored.id);

    expect(deleteMock).toHaveBeenCalledWith(stored.id);
    expect(words.words).toHaveLength(0);
  });
});
