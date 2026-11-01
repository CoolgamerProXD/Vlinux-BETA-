/**
 * A clock that wakes up only when the displayed value can change
 * (next minute boundary, or next second when seconds are shown).
 */
class Clock {
  now = $state(new Date());
  private timer: ReturnType<typeof setTimeout> | undefined;
  private perSecond = false;

  start(perSecond: boolean) {
    this.perSecond = perSecond;
    this.stop();
    this.tick();
  }

  stop() {
    clearTimeout(this.timer);
  }

  private tick = () => {
    // Use a local value: reading `this.now` here would make any calling $effect depend on
    // the state it is writing and loop forever.
    const now = new Date();
    this.now = now;
    const ms = this.perSecond ? 1000 - now.getMilliseconds() : 60_000 - (now.getSeconds() * 1000 + now.getMilliseconds());
    this.timer = setTimeout(this.tick, Math.max(50, ms));
  };
}

export const clock = new Clock();
