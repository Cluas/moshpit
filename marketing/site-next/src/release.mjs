/**
 * Where the product is in its release, in one place.
 *
 * The site has to say three different things over about two weeks — beta is in
 * review, beta is open, the app is on sale — and each of those appears in the
 * hero, in the pricing page and in the footer of both languages. That is twelve
 * places to forget one. On launch day the intended edit is this file only.
 *
 * `stage` is the switch. Everything else is data the stages read.
 */
export const RELEASE = {
  /** 'review' → beta submitted, no link yet · 'beta' → link live · 'sale' → on the App Store */
  stage: 'review',

  /**
   * The public TestFlight invite. Stays null until Beta App Review passes
   * (~24h) and Wenlong hands over the link — a button that goes nowhere is
   * worse than a button that is not there yet, so `stage: 'beta'` without this
   * throws rather than rendering a dead link.
   */
  testflightUrl: null,

  /** Assigned by App Store Connect; the store URL is derived from it. */
  appStoreId: '6799896801',

  /** Launch price. The ladder and its triggers live in docs/appstore/pricing.md. */
  price: '$6.99',
};

const COPY = {
  en: {
    review: { note: `<b>${RELEASE.price}</b> one-time · TestFlight beta in review` },
    beta: { button: 'Join the beta (TestFlight)', note: `Free while in beta · <b>${RELEASE.price}</b> one-time at launch` },
    sale: { button: 'Download on the App Store', note: `<b>${RELEASE.price}</b> one-time · no subscription` },
  },
  zh: {
    review: { note: `<b>${RELEASE.price}</b> 一次性买断 · TestFlight 外测审核中` },
    beta: { button: '加入 TestFlight 外测', note: `外测期间免费 · 上架后 <b>${RELEASE.price}</b> 一次性买断` },
    sale: { button: '在 App Store 下载', note: `<b>${RELEASE.price}</b> 一次性买断 · 无订阅` },
  },
};

/**
 * The primary call to action, as HTML.
 *
 * Returns an empty string in the review stage on purpose: the hero's secondary
 * buttons ("See it end to end", "How it compares") are real and useful, and a
 * disabled primary button next to them would only advertise that we are not
 * ready.
 */
export function ctaPrimary(lang = 'en') {
  const copy = COPY[lang][RELEASE.stage];
  if (RELEASE.stage === 'review') return '';
  if (RELEASE.stage === 'beta') {
    if (!RELEASE.testflightUrl) {
      throw new Error("RELEASE.stage is 'beta' but testflightUrl is null — that would render a dead button.");
    }
    return `<a class="cta" href="${RELEASE.testflightUrl}">${copy.button}</a>`;
  }
  return `<a class="cta" href="https://apps.apple.com/app/id${RELEASE.appStoreId}">${copy.button}</a>`;
}

/**
 * Whether a primary button is rendered at all.
 *
 * The hero's other buttons read the answer: while the beta link does not exist
 * there is no filled button, so "See it end to end" takes the filled slot
 * rather than leaving the hero with two identical outlines and no focal point.
 */
export const hasPrimaryCta = RELEASE.stage !== 'review';

/** The one-line price/availability note that sits under the buttons. */
export function ctaNote(lang = 'en') {
  return COPY[lang][RELEASE.stage].note;
}
