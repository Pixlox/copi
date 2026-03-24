import { check, Update } from "@tauri-apps/plugin-updater";
import { ask, message } from "@tauri-apps/plugin-dialog";
import { relaunch } from "@tauri-apps/plugin-process";

export async function checkForUpdates(silent: boolean = false): Promise<void> {
  try {
    const update: Update | null = await check();

    if (!update) {
      console.log("[Updater] No update available");
      return;
    }

    console.log(`[Updater] Update available: ${update.version}`);

    if (silent) {
      // Auto-update: download and install without prompt
      await downloadAndInstall(update);
      return;
    }

    // Ask user
    const userConfirmed = await ask(
      `Version ${update.version} is available.\n\n${update.body ?? ""}\n\nUpdate now?`,
      {
        title: "Update Available",
        kind: "info",
      }
    );

    if (!userConfirmed) return;

    await downloadAndInstall(update);
  } catch (error) {
    console.error("[Updater] Failed:", error);
    if (!silent) {
      await message(`Update check failed: ${error}`, {
        title: "Update Error",
        kind: "error",
      });
    }
  }
}

async function downloadAndInstall(update: Update): Promise<void> {
  let downloaded = 0;
  let contentLength = 0;

  await update.downloadAndInstall((event) => {
    switch (event.event) {
      case "Started":
        contentLength = event.data.contentLength ?? 0;
        console.log(`[Updater] Download started: ${contentLength} bytes`);
        break;
      case "Progress":
        downloaded += event.data.chunkLength;
        break;
      case "Finished":
        console.log("[Updater] Download finished");
        break;
    }
  });

  const shouldRelaunch = await ask(
    "Update installed. Restart now to apply?",
    { title: "Restart Required", kind: "info" }
  );

  if (shouldRelaunch) {
    await relaunch();
  }
}
