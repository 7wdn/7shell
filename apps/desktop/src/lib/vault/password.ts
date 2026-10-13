/**
 * What the interface asks of a vault password, before it asks the backend to use one.
 *
 * A rule of the forms rather than of the store: the backend derives a key from whatever it is given,
 * so a one-character master password would be accepted and would be a one-character master password.
 * Both places a password is *chosen* — creating a vault and changing its master password — hold to
 * this, and neither of them is the place for a second copy of the number.
 *
 * It is deliberately not the same rule as a file's passphrase (`MIN_PASSPHRASE_LEN`, four
 * characters, in the crate): that one guards an export somebody is about to hand to a colleague, and
 * arguing it up there would be arguing about somebody else's file.
 */
export const MINIMUM_PASSWORD_LENGTH = 8;
