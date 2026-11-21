import type { Tab } from "@/contexts/TabContext";
import { t as translate } from "./i18n";

// Keep titles in the saved tab data language-neutral. Translate only built-in titles.
export function getTabDisplayTitle(tab: Tab): string {
  switch (tab.type) {
    case "projects":
    case "agents":
    case "usage":
    case "mcp":
    case "settings":
    case "create-agent":
    case "import-agent":
      return translate(tab.title);
    case "agent-execution":
      return tab.agentData?.name ? translate("Run: {v1}", { v1: tab.agentData.name }) : tab.title;
    case "chat": {
      const match = !tab.initialProjectPath && !tab.sessionId && /^Chat (\d+)$/.exec(tab.title);
      return match ? translate("Chat {number}", { number: match[1] }) : tab.title;
    }
    default:
      return tab.title;
  }
}
