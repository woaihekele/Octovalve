export function isMacPlatform(): boolean {
  if (typeof navigator === 'undefined') {
    return false;
  }
  const platform =
    (navigator as { userAgentData?: { platform?: string } }).userAgentData?.platform ||
    navigator.platform ||
    navigator.userAgent;
  return /mac|iphone|ipad|ipod/i.test(platform);
}
