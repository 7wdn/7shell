/**
 * Reading a directory off a command line, and knowing when not to.
 *
 * The refusals are the point of this suite. Following a `cd` that the shell did not actually run
 * moves a file panel somewhere nobody asked for, and the person has no way to tell that it was a
 * guess — so every case below that ends in `null` is a case where the line does not surely name a
 * directory, and staying put is the answer.
 */
import { describe, expect, it } from "vitest";

import { TypedCommands, directoryFromOsc7, directoryInCommand, resolveDirectory } from "@/lib/terminal/follow";

describe("the directory a command line names", () => {
  it("reads a plain absolute path", () => {
    expect(directoryInCommand("cd /data")).toBe("/data");
    expect(directoryInCommand("  cd   /data  ")).toBe("/data");
    expect(directoryInCommand("cd /data/")).toBe("/data/");
  });

  it("reads a relative path, which is resolved later, and the home shorthand", () => {
    expect(directoryInCommand("cd ..")).toBe("..");
    expect(directoryInCommand("cd ~/work")).toBe("~/work");
    expect(directoryInCommand("cd ~")).toBe("~");
  });

  it("reads a bare cd, which means the home directory", () => {
    expect(directoryInCommand("cd")).toBe("");
    expect(directoryInCommand("cd   ")).toBe("");
  });

  it("takes the quoting off a path with spaces in it", () => {
    expect(directoryInCommand(`cd "/my dir"`)).toBe("/my dir");
    expect(directoryInCommand(`cd '/my dir'`)).toBe("/my dir");
  });

  it("takes a single-quoted path exactly as written, expansion and all", () => {
    expect(directoryInCommand(`cd '/my $dir'`)).toBe("/my $dir");
    expect(directoryInCommand(`cd '/a;b'`)).toBe("/a;b");
  });

  it("refuses a path the shell has to work out", () => {
    expect(directoryInCommand("cd $HOME")).toBeNull();
    expect(directoryInCommand('cd "/my $dir"')).toBeNull();
    expect(directoryInCommand("cd $(pwd)/x")).toBeNull();
    expect(directoryInCommand("cd ../x*")).toBeNull();
    expect(directoryInCommand("cd /my\\ dir")).toBeNull();
    expect(directoryInCommand("cd ~/../x")).toBe("~/../x");
  });

  it("refuses a line that is not a cd at all", () => {
    expect(directoryInCommand("ls -la")).toBeNull();
    expect(directoryInCommand("")).toBeNull();
    expect(directoryInCommand("cdx /data")).toBeNull();
    expect(directoryInCommand("sudo cd /data")).toBeNull();
    expect(directoryInCommand("hunter2")).toBeNull();
  });

  it("refuses arguments the shell would read as more than a path", () => {
    // Two arguments: `cd /my dir` is refused by the shell itself.
    expect(directoryInCommand("cd /my dir")).toBeNull();
    // A flag, which the shell accepts and this side cannot interpret.
    expect(directoryInCommand("cd -P /data")).toBeNull();
    // A comment, which the shell strips.
    expect(directoryInCommand("cd /data # here")).toBeNull();
  });

  it("follows the cd that runs, not the one that ran first", () => {
    expect(directoryInCommand("cd /data && ls")).toBe("/data");
    expect(directoryInCommand("cd /data; ls")).toBe("/data");
    expect(directoryInCommand("ls; cd /data")).toBe("/data");
    expect(directoryInCommand("cd /a; cd /b")).toBe("/b");
    expect(directoryInCommand("cd /a && cd /b")).toBe("/b");
  });

  it("gives up when a later cd is one it cannot read, rather than following the earlier one", () => {
    // The shell is wherever `$X` points, which is not /a and not knowable.
    expect(directoryInCommand("cd /a; cd $X")).toBeNull();
  });

  it("refuses a cd that runs in a subshell, because the shell does not move", () => {
    expect(directoryInCommand("cd /data &")).toBeNull();
    expect(directoryInCommand("cd /data | cat")).toBeNull();
    expect(directoryInCommand("cd /a; tail -f x | grep y")).toBe("/a");
  });

  it("takes a separator inside quotes as part of the path", () => {
    expect(directoryInCommand(`cd "/a;b"`)).toBe("/a;b");
  });
});

describe("the directory an argument resolves to", () => {
  it("leaves an absolute path alone", () => {
    expect(resolveDirectory("/home/user", "/home/user", "/data")).toBe("/data");
  });

  it("resolves a relative path against where the shell is, not where the browser is showing", () => {
    // The browser can be browsed by hand while the shell stays put; a `cd logs` belongs to the
    // shell's directory.
    expect(resolveDirectory("/srv/app", "/home/user", "logs")).toBe("/srv/app/logs");
    expect(resolveDirectory("/srv/app/", "/home/user", "logs")).toBe("/srv/app/logs");
    expect(resolveDirectory("/srv/app", "/home/user", "../logs")).toBe("/srv/app/../logs");
  });

  it("takes the home shorthand to where the browser opened, which is where the shell started", () => {
    expect(resolveDirectory("/srv/app", "/home/user", "")).toBe("/home/user");
    expect(resolveDirectory("/srv/app", "/home/user", "~")).toBe("/home/user");
    expect(resolveDirectory("/srv/app", "/home/user", "~/")).toBe("/home/user");
    expect(resolveDirectory("/srv/app", "/home/user", "~/work")).toBe("/home/user/work");
  });

  it("refuses the two directories it cannot know", () => {
    // Somebody else's home is in the password database on the far side, not here.
    expect(resolveDirectory("/home/user", "/home/user", "~other/x")).toBeNull();
    // The shell's OLDPWD is the shell's, and the browser's previous directory is not the same thing.
    expect(resolveDirectory("/srv/app", "/home/user", "-")).toBeNull();
  });
});

describe("the directory a shell reports", () => {
  it("reads what OSC 7 carries", () => {
    expect(directoryFromOsc7("file://host/home/user")).toBe("/home/user");
    expect(directoryFromOsc7("file:///home/user")).toBe("/home/user");
    expect(directoryFromOsc7("file://host/home/my%20dir")).toBe("/home/my dir");
  });

  it("refuses anything else, rather than guessing at it", () => {
    expect(directoryFromOsc7("file://host")).toBeNull();
    expect(directoryFromOsc7("/home/user")).toBeNull();
    expect(directoryFromOsc7("http://host/home/user")).toBeNull();
    expect(directoryFromOsc7("file://host/%E0%A4%A")).toBeNull();
  });
});

describe("reading the line somebody is typing", () => {
  it("says nothing until a line is complete", () => {
    const typed = new TypedCommands();

    expect(typed.feed("cd /data")).toBeNull();
    expect(typed.feed("\r")).toBe("/data");
  });

  it("lets a typo be erased before Enter", () => {
    const typed = new TypedCommands();
    typed.feed("cd /datz");

    typed.feed("\x7f");
    typed.feed("\x7f");
    expect(typed.feed("ta\r")).toBe("/data");
  });

  it("refuses a quoted empty name, which is not the home directory", () => {
    expect(directoryInCommand("cd ''")).toBeNull();
    expect(directoryInCommand(`cd ""`)).toBeNull();
  });

  it("knows the three ways to throw a line away", () => {
    const killed = new TypedCommands();
    expect(killed.feed("cd /nope\x15cd /data\r")).toBe("/data");

    const abandoned = new TypedCommands();
    expect(abandoned.feed("cd /nope\x03")).toBeNull();
    expect(abandoned.feed("cd /data\r")).toBe("/data");

    const word = new TypedCommands();
    expect(word.feed("cd /nope /data\x17\r")).toBe("/nope");
  });

  it("gives up on a line a completion or a history recall rewrote", () => {
    const completed = new TypedCommands();
    expect(completed.feed("cd /da\t\r")).toBeNull();

    const recalled = new TypedCommands();
    expect(recalled.feed("cd /da\x1b[A\r")).toBeNull();

    // …and is ready to read the next line, which was typed in full.
    const next = new TypedCommands();
    next.feed("cd /da\x1b[A\r");
    expect(next.feed("cd /data\r")).toBe("/data");
  });

  it("reads a pasted command, including one that was pasted as several lines", () => {
    const pasted = new TypedCommands();
    expect(pasted.feed("\x1b[200~cd /data\x1b[201~\r")).toBe("/data");

    const script = new TypedCommands();
    expect(script.feed("\x1b[200~cd /data\nls -la\x1b[201~\r")).toBe("/data");
  });

  it("says nothing about a line that is not a cd", () => {
    const typed = new TypedCommands();
    expect(typed.feed("hunter2\r")).toBeNull();
    expect(typed.feed("\r")).toBeNull();
  });
});
