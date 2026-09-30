/**
 * Class name helper, the convention every component in this project follows:
 * `cn("px-2", isActive && "bg-accent")` — later classes win, conflicts are resolved by
 * tailwind-merge rather than by source order.
 */
import { type ClassValue, clsx } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]): string {
  return twMerge(clsx(inputs));
}
