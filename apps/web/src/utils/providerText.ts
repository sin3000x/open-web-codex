const LEADING_PROVIDER_SENTINELS = [
  "<｜begin▁of▁sentence｜>",
  "<|begin_of_sentence|>",
  "<｜begin_of_sentence｜>",
];

export function stripLeadingProviderSentinel(text: string) {
  let next = text;
  let didStrip = true;
  while (didStrip) {
    didStrip = false;
    const leadingWhitespace = next.match(/^\s*/)?.[0] ?? "";
    const body = next.slice(leadingWhitespace.length);
    for (const sentinel of LEADING_PROVIDER_SENTINELS) {
      if (body.startsWith(sentinel)) {
        next = `${leadingWhitespace}${body.slice(sentinel.length)}`;
        didStrip = true;
        break;
      }
    }
  }
  return next;
}
