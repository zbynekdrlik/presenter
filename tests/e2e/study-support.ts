/**
 * Shared by the `study-*.spec.ts` files (#851): open the page on a device
 * profile, settle on a chapter, and watch that nothing reaches a wall (T11).
 */
import {
  expect,
  type Browser,
  type BrowserContext,
  type BrowserContextOptions,
  type Locator,
  type Page,
  type Response,
} from "@playwright/test";
import { attachConsoleErrorCollector } from "./support";

export type OpenedStudy = {
  context: BrowserContext;
  page: Page;
  errors: string[];
  wallRequests: string[];
};

/** T11: the paths that could change a wall. */
const WALL_PATH = /^\/bible\/(trigger|clear)|^\/stage\/|^\/bible\/presentations\/.*\/trigger/;

export function watchWallRequests(page: Page): string[] {
  const seen: string[] = [];
  page.on("request", (request) => {
    const path = new URL(request.url()).pathname;
    if (WALL_PATH.test(path)) seen.push(`${request.method()} ${path}`);
  });
  return seen;
}

export const chapter = (page: Page): Locator => page.locator('[data-role="study-chapter"]');
export const field = (page: Page): Locator => page.locator('[data-role="study-field"]');
export const verse = (page: Page, number: number): Locator =>
  page.locator(`[data-role="study-verse"][data-verse="${number}"]`);
export const verseNumber = (page: Page, number: number): Locator =>
  verse(page, number).locator('[data-role="study-verse-number"]');
export const favourite = (page: Page, code: string): Locator =>
  page.locator(`[data-role="study-fav"][data-translation="${code}"]`);

/** The settle marker: the render that shows it shows that chapter. */
export async function expectChapter(page: Page, key: string): Promise<void> {
  await expect(chapter(page)).toHaveAttribute("data-loaded-for", key);
}

/** Holds every matching request until the returned `release` is called. */
export async function gateRoute(page: Page, url: string): Promise<() => void> {
  let release!: () => void;
  const gate = new Promise<void>((resolve) => (release = resolve));
  await page.route(url, async (route) => {
    await gate;
    await route.fallback();
  });
  return release;
}

/** Waits until a response's body has fully arrived (headers alone resolve `waitForResponse`). */
export async function landed(response: Promise<Response>): Promise<void> {
  await (await response).finished();
}

export async function openStudy(
  browser: Browser,
  baseURL: string,
  profile: BrowserContextOptions,
  path = "/ui/study?t=slk-seb&p=HEB.11",
  loadedFor: string | null = "slk-seb:HEB.11",
  prepare?: (page: Page) => Promise<void>,
): Promise<OpenedStudy> {
  const context = await browser.newContext({ ...profile, baseURL });
  const page = await context.newPage();
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  const wallRequests = watchWallRequests(page);
  if (prepare) await prepare(page);
  await page.goto(path);
  await page.waitForSelector('body[data-wasm-ready="true"]', { state: "attached", timeout: 30_000 });
  if (loadedFor) await expectChapter(page, loadedFor);
  return { context, page, errors, wallRequests };
}

/** T10 + T11, the last assertions of every test. */
export async function finish(opened: OpenedStudy): Promise<void> {
  expect(opened.wallRequests, "nothing may reach a wall").toEqual([]);
  expect(opened.errors, "browser console").toEqual([]);
  await opened.context.close();
}
