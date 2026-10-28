import { describe, expect, it } from 'vitest';
import { BridgeUptimeWindow } from './bridge-uptime';

describe('本机连接连着的时间比例', () => {
  it('按时间算：启动那几秒不会把比例拉到一半', () => {
    const uptime = new BridgeUptimeWindow();
    uptime.observe('starting', 0);
    uptime.observe('ready', 3_000);
    expect(uptime.take(900_000)).toBeCloseTo(897 / 900);
  });

  it('每取一次从头算，断开的那一段只算在它那一段里', () => {
    const uptime = new BridgeUptimeWindow();
    uptime.observe('authorized', 0);
    expect(uptime.take(900_000)).toBe(1);
    uptime.observe('reconnecting', 1_200_000);
    uptime.observe('ready', 1_500_000);
    expect(uptime.take(1_800_000)).toBeCloseTo(600 / 900);
  });

  it('还没授权、停下了、还在找 Bridge 都不算；一直这样就不报', () => {
    const uptime = new BridgeUptimeWindow();
    expect(uptime.take(1_000)).toBeNull();
    uptime.observe('discovering', 1_000);
    uptime.observe('authorization_required', 5_000);
    expect(uptime.take(900_000)).toBeNull();
    uptime.observe('ready', 900_000);
    uptime.observe('stopped', 1_000_000);
    expect(uptime.take(1_800_000)).toBe(1);
  });

  it('停住的连接算没连上', () => {
    const uptime = new BridgeUptimeWindow();
    uptime.observe('halted', 0);
    expect(uptime.take(900_000)).toBe(0);
  });
});
