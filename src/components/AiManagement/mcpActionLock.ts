export class ExclusiveActionLock {
  private activeKey: string | null = null;

  acquire(key: string): boolean {
    if (this.activeKey !== null) return false;
    this.activeKey = key;
    return true;
  }

  release(key: string): boolean {
    if (this.activeKey !== key) return false;
    this.activeKey = null;
    return true;
  }

  get current(): string | null {
    return this.activeKey;
  }
}
