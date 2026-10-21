import { withBasePath } from "../site/base-path";

const TIMEOUT_MS = 10_000;

/** A static file of the site, checked by `guard`; fails after `TIMEOUT_MS` or when `signal` aborts. */
export async function fetchJson<T>(path: string, guard: (v: unknown) => v is T, signal: AbortSignal): Promise<T> {
  const res = await fetch(withBasePath(path), { signal: AbortSignal.any([signal, AbortSignal.timeout(TIMEOUT_MS)]) });
  if (!res.ok) throw new Error(`${res.status} ${res.statusText}`);
  const data: unknown = await res.json();
  if (!guard(data)) throw new Error(`${path} is not in the expected shape`);
  return data;
}
