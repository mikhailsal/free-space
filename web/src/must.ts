/** `v` itself; throws, naming `what`, when it is null or undefined. For what the
 * page's state guarantees but its types cannot show (the main page's elements,
 * the status it was drawn from). */
export function must<T>(v: T | null | undefined, what: string): T {
  if (v === null || v === undefined) throw new Error(`${what} is missing`);
  return v;
}
