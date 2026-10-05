/**
 * Who claims what, and what nobody claims yet.
 *
 * A capability is claimed when *any* descriptor claims it — a protocol, or an authentication
 * method. Asking only the protocols is what made the home page report stored secrets,
 * keyboard-interactive and key files as unclaimed while all three were built and declared: the
 * vocabulary is shared, so "is this real" has to be asked of everything that can answer.
 */

import {
  AUTH_METHODS,
  CAPABILITIES,
  PROTOCOLS,
  type CapabilityDescriptor,
  type CapabilityKey,
} from "@/types/generated/protocols";

export function isCapabilityClaimed(capability: CapabilityKey): boolean {
  return (
    PROTOCOLS.some((protocol) => protocol.capabilities.includes(capability)) ||
    AUTH_METHODS.some((method) => method.capabilities.includes(capability))
  );
}

/**
 * What this build can do, according to the same table Rust compiles against.
 *
 * The descriptors themselves rather than their keys: what draws them needs the label and the order
 * as well, and mapping back to the spec at the call site is the same lookup done twice.
 */
export function claimedCapabilities(): readonly CapabilityDescriptor[] {
  return CAPABILITIES.filter((capability) => isCapabilityClaimed(capability.key));
}

/**
 * What is in the vocabulary and nothing claims.
 *
 * Shown on the home page on purpose: a capability nobody claims is a thing this build does not do,
 * and saying so is better than an interface that quietly offers it.
 */
export function unclaimedCapabilities(): readonly CapabilityDescriptor[] {
  return CAPABILITIES.filter((capability) => !isCapabilityClaimed(capability.key));
}
