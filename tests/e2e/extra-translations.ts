/**
 * #921: SNV has ten translations, CI ingests six (KJV, SEB, ROH, SEVP, MIL +
 * the remote NLT). The translation rows must stay ONE line with ten, so these
 * specs list four more the way SNV does (AMPC, BOT, SSV, CSP) and answer their
 * reads with the KJV's (the same book codes): the UI under test only switches
 * translations and lays them out; the real texts are read live on SNV.
 */
import type { Page } from "@playwright/test";

export const EXTRA_TRANSLATIONS = [
  { code: "eng-ampc", name: "Amplified Bible, Classic Edition", language: "en" },
  { code: "slk-bot", name: "Jeruzalemská Biblia (Botek)", language: "sk" },
  { code: "slk-ssv", name: "Katolícky preklad", language: "sk" },
  { code: "ces-csp", name: "Český studijní překlad", language: "cs" },
] as const;

const EXTRA_CODES = new Set<string>(EXTRA_TRANSLATIONS.map((t) => t.code));

/** Lists the four extra translations and serves their reads from the KJV. */
export async function routeExtraTranslations(page: Page): Promise<void> {
  await page.route(
    (url) => url.pathname === "/bible/translations",
    async (route) => {
      const response = await route.fetch();
      const list = (await response.json()) as { code: string }[];
      const extra = EXTRA_TRANSLATIONS.map((t) => ({ ...t, showInDashboard: true }));
      await route.fulfill({ response, json: [...list, ...extra] });
    },
  );
  await page.route(
    (url) => EXTRA_CODES.has(url.searchParams.get("translation") ?? ""),
    async (route) => {
      const url = new URL(route.request().url());
      url.searchParams.set("translation", "eng-kjv");
      await route.continue({ url: url.toString() });
    },
  );
}
