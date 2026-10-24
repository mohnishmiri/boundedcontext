type Listener = (ev: Event) => void;

/** Minimal controllable EventSource for tests (`vi.stubGlobal('EventSource', MockEventSource)`). */
export class MockEventSource {
  static instances: MockEventSource[] = [];

  static reset(): void {
    MockEventSource.instances = [];
  }

  static latest(): MockEventSource {
    const es = MockEventSource.instances[MockEventSource.instances.length - 1];
    if (!es) throw new Error('no EventSource was opened');
    return es;
  }

  readonly url: string;
  readyState = 0;
  closed = false;
  private listeners = new Map<string, Set<Listener>>();

  constructor(url: string) {
    this.url = url;
    MockEventSource.instances.push(this);
  }

  addEventListener(type: string, fn: Listener): void {
    if (!this.listeners.has(type)) this.listeners.set(type, new Set());
    this.listeners.get(type)?.add(fn);
  }

  removeEventListener(type: string, fn: Listener): void {
    this.listeners.get(type)?.delete(fn);
  }

  close(): void {
    this.closed = true;
    this.readyState = 2;
  }

  emit(type: string, data?: unknown): void {
    if (type === 'open') this.readyState = 1;
    const ev = data === undefined ? new Event(type) : new MessageEvent(type, { data: JSON.stringify(data) });
    for (const fn of this.listeners.get(type) ?? []) fn(ev);
  }
}
