/**
 * A Cache Storage of the tests' own, in memory, with the parts of the API the
 * offline module and the service worker use: enough to say what was stored,
 * under which address, and what is answered back.
 */
const BASE = "http://client.test";

const absolute = (key: RequestInfo | URL) =>
  new URL(
    typeof key === "string" ? key : key instanceof URL ? key.href : key.url,
    BASE,
  ).href;

export class FakeCache {
  readonly entries = new Map<string, Response>();

  async put(key: RequestInfo | URL, response: Response): Promise<void> {
    this.entries.set(absolute(key), response.clone());
  }

  async match(key: RequestInfo | URL): Promise<Response | undefined> {
    return this.entries.get(absolute(key))?.clone();
  }

  async delete(key: RequestInfo | URL): Promise<boolean> {
    return this.entries.delete(absolute(key));
  }

  async keys(): Promise<Request[]> {
    return [...this.entries.keys()].map((url) => new Request(url));
  }
}

export class FakeCaches {
  readonly stores = new Map<string, FakeCache>();

  async open(name: string): Promise<FakeCache> {
    let cache = this.stores.get(name);
    if (!cache) {
      cache = new FakeCache();
      this.stores.set(name, cache);
    }
    return cache;
  }

  async match(
    key: RequestInfo | URL,
    options?: { cacheName?: string },
  ): Promise<Response | undefined> {
    const names = options?.cacheName
      ? [options.cacheName]
      : [...this.stores.keys()];
    for (const name of names) {
      const found = await this.stores.get(name)?.match(key);
      if (found) return found;
    }
    return undefined;
  }

  async keys(): Promise<string[]> {
    return [...this.stores.keys()];
  }

  async delete(name: string): Promise<boolean> {
    return this.stores.delete(name);
  }

  /** Every address stored anywhere, path only. */
  paths(): string[] {
    return [...this.stores.values()].flatMap((cache) =>
      [...cache.entries.keys()].map((url) => {
        const parsed = new URL(url);
        return parsed.pathname + parsed.search;
      }),
    );
  }
}
