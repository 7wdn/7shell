/**
 * The group a command lands in when nobody has made one yet.
 *
 * The same id the storage uses. It is here as a constant because the interface has to *not* offer
 * to delete it: it is where the next saved command goes, and a library with nowhere to put one is
 * a library that has to invent the group back.
 */
export const DEFAULT_GROUP_ID = "default";
