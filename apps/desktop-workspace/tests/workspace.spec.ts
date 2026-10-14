import { test, expect, type Page } from "@playwright/test";
const audio = (page: Page) =>
  page.getByRole("region", { name: "Desktop Control 패널" });
const windows = (page: Page) =>
  page.getByRole("region", { name: "Window Manager 패널" });

test.beforeEach(async ({ page }) => {
  await page.goto("/?mode=fixture");
  await expect(audio(page)).toBeVisible();
});
test("single selection is visible and default aliases cannot route into themselves", async ({
  page,
}) => {
  const source = audio(page).getByRole("group", { name: /^원본 출력/ });
  const target = audio(page).getByRole("group", { name: /^대상 출력/ });
  await expect(
    source.getByRole("radio", { name: /^Windows 기본 출력/ }),
  ).toBeChecked();
  await source.getByRole("radio", { name: /^헤드폰/ }).check();
  await expect(source.locator(".selected-badge")).toHaveText("선택됨");
  await expect(
    source.getByRole("radio", { name: /^Windows 기본 출력/ }),
  ).not.toBeChecked();
  await target.getByRole("radio", { name: /^헤드폰/ }).check();
  await expect(audio(page).getByRole("alert")).toContainText(
    "원본과 대상이 같은 출력",
  );
  await expect(
    audio(page).getByRole("button", { name: "테스트 라우팅 시작" }),
  ).toBeDisabled();
});
test("exclusive error includes remedy and request ID in both panel and diagnostics", async ({
  page,
}) => {
  await audio(page).getByLabel("오류 재현 시나리오").selectOption("exclusive");
  await audio(page).getByRole("button", { name: "테스트 라우팅 시작" }).click();
  const issue = audio(page).getByRole("alert");
  await expect(issue).toContainText("0x8889000A");
  await expect(issue).toContainText("공유 모드");
  await expect(issue).toContainText("fixture-");
  await page.getByLabel("추가할 pane 구성요소").selectOption("diagnostics");
  await page.getByRole("button", { name: "pane 추가", exact: true }).click();
  await expect(page.getByRole("log", { name: "명령 기록" })).toContainText(
    "AUDCLNT_E_DEVICE_IN_USE",
  );
});
test("window application requires a fresh preview and affects only selected mock windows", async ({
  page,
}) => {
  await page
    .locator(".dv-tab")
    .filter({ hasText: "Window Manager" })
    .first()
    .click();
  const panel = windows(page);
  await expect(
    panel.getByRole("button", { name: "가상 창에 적용" }),
  ).toBeDisabled();
  await panel.getByRole("button", { name: "배치 미리보기" }).click();
  await expect(panel.getByLabel("배치 미리보기 결과")).toContainText("2개 창");
  await panel.getByRole("checkbox", { name: /^개발 서버/ }).check();
  await expect(
    panel.getByRole("button", { name: "가상 창에 적용" }),
  ).toBeDisabled();
  await panel.getByRole("button", { name: "배치 미리보기" }).click();
  await panel.getByRole("button", { name: "가상 창에 적용" }).click();
  await expect(panel.getByRole("status")).toContainText("3개 가상 창");
});
test("popout and redocking preserve shared state and running mock jobs", async ({
  page,
}) => {
  const opened = page.waitForEvent("popup");
  await audio(page).getByRole("button", { name: "새 창으로 이동" }).click();
  const popup = await opened;
  await expect(audio(popup)).toBeVisible();
  await audio(popup)
    .getByRole("group", { name: /^원본 출력/ })
    .getByRole("radio", { name: /^헤드폰/ })
    .check();
  await audio(popup)
    .getByRole("button", { name: "테스트 라우팅 시작" })
    .click();
  await expect(
    audio(popup).getByText("테스트 실행 중", { exact: true }),
  ).toBeVisible();
  await audio(popup).getByRole("button", { name: "주 창으로 합치기" }).click();
  await expect(audio(page)).toBeVisible();
  await expect(
    audio(page)
      .getByRole("group", { name: /^원본 출력/ })
      .getByRole("radio", { name: /^헤드폰/ }),
  ).toBeChecked();
  await expect(
    audio(page).getByText("테스트 실행 중", { exact: true }),
  ).toBeVisible();
});
test("tab dragging changes groups and the arrangement survives reload", async ({
  page,
}) => {
  const source = page
    .locator(".dv-tab")
    .filter({ hasText: "Audio Repeater" })
    .first();
  const target = page
    .locator(".dv-tab")
    .filter({ hasText: "Window Manager" })
    .first();
  await source.dragTo(target);
  await expect
    .poll(() =>
      page.evaluate(() => {
        const layout = JSON.parse(
          localStorage.getItem("desktop-workspace.tabs.v2") ?? "{}",
        );
        const groups: string[][] = [];
        const walk = (node: { type: string; data: unknown }) => {
          if (node.type === "leaf")
            groups.push((node.data as { views: string[] }).views);
          else
            for (const child of node.data as { type: string; data: unknown }[])
              walk(child);
        };
        if (layout.grid) walk(layout.grid.root);
        return groups.some(
          (group) => group.includes("audio") && group.includes("windows"),
        );
      }),
    )
    .toBe(true);
  await page.reload();
  await expect(audio(page)).toBeVisible();
  await expect(
    page.locator(".dv-tab").filter({ hasText: "Audio Cable" }).first(),
  ).toBeVisible();
});
test("corrupt saved layout falls back without crashing", async ({ page }) => {
  await page.evaluate(() =>
    localStorage.setItem("desktop-workspace.tabs.v2", "bad-json"),
  );
  await page.reload();
  await expect(audio(page)).toBeVisible();
  await expect(
    page.getByRole("button", { name: "탭 추가", exact: true }),
  ).toBeVisible();
});
test("workspace screenshot", async ({ page }) => {
  await page.screenshot({ path: "test-results/workspace.png", fullPage: true });
});
