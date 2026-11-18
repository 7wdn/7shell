/**
 * Which kinds of session have a connection the test button can answer.
 *
 * SSH is the full test — route, host key and credential. Telnet and a raw TCP console have no
 * login, so for them the test is the address answering. Everything else opens as a folder, a
 * desktop, a container workspace or a local process, and a "test" there would be a promise this
 * feature cannot keep.
 */
import type { Protocol } from "@/types/generated/protocols";

const TESTABLE: readonly Protocol[] = ["ssh", "telnet", "rawTcp"];

export function canTestConnection(protocol: Protocol): boolean {
  return TESTABLE.includes(protocol);
}
