import "./Avatar.css";

export type AvatarSize = "xs" | "sm" | "md" | "lg";

export interface AvatarProps {
  /** The display name, which is where the initials come from. */
  name: string;
  /** The address. The hue is chosen from this when it is known, so a person keeps their colour
   *  even when a message carries their name differently. */
  address?: string;
  /** A company rather than a person: a bordered mark in the text face, no hue. */
  brand?: boolean;
  /**
   * Overrides the hue derived from the address. An account carries a colour somebody chose in
   * settings, and the account chip has to wear that one rather than whichever the address hashes
   * to; everyone else keeps the derived hue, so a sender is the same colour on every device.
   */
  hue?: number;
  /** xs is a compose chip, sm a stacked participant, md a list row, lg a contact card. */
  size?: AvatarSize;
}

/** Eight hues, chosen from the address so the same sender is the same colour on every device. */
export function avatarHue(key: string): number {
  let h = 0;
  for (let i = 0; i < key.length; i++) h = (h * 31 + key.charCodeAt(i)) >>> 0;
  return (h % 8) + 1;
}

/** Two letters: the initials of a person, the first syllable of a company. */
export function avatarInitials(name: string, brand?: boolean): string {
  const words = name.trim().split(/\s+/).filter(Boolean);
  if (words.length === 0) return "?";
  if (brand) {
    const w = words[0];
    return (w[0] ?? "").toUpperCase() + (w[1] ?? "").toLowerCase();
  }
  if (words.length === 1) return (words[0][0] ?? "").toUpperCase();
  return ((words[0][0] ?? "") + (words[words.length - 1][0] ?? "")).toUpperCase();
}

export function Avatar({ name, address, brand, hue: chosen, size = "md" }: AvatarProps) {
  const hue = chosen ?? avatarHue(address ?? name);
  return (
    <span
      className="avatar"
      data-size={size}
      data-hue={brand ? undefined : hue}
      data-brand={brand ? "" : undefined}
      aria-hidden="true"
    >
      {avatarInitials(name, brand)}
    </span>
  );
}

export interface AvatarStackProps {
  people: { name: string; address?: string; brand?: boolean }[];
  size?: AvatarSize;
  /** How many faces to draw before the rest become one chip. */
  max?: number;
}

/**
 * As many faces as the cap allows, and past it the remainder as a count in the same circle.
 *
 * The cap is here rather than in whichever screen happens to be drawing, because an unbounded row
 * of circles is a property of the stack: a calendar invite addressed to forty people put forty of
 * them across the head of the thread and left the message under the fold. Four is the number at
 * which faces stop being people you recognise and start being a texture.
 */
export function AvatarStack({ people, size = "sm", max = 4 }: AvatarStackProps) {
  const faces = people.length > max ? people.slice(0, max) : people;
  const rest = people.length - faces.length;
  return (
    <span className="avatar-stack">
      {faces.map((p) => (
        <Avatar key={p.address ?? p.name} name={p.name} address={p.address} brand={p.brand} size={size} />
      ))}
      {rest > 0 ? (
        <span className="avatar" data-size={size} data-more="" aria-hidden="true">
          {`+${rest}`}
        </span>
      ) : null}
    </span>
  );
}
