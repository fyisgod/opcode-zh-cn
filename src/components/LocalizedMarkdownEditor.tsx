import type { ComponentProps } from "react";
import MDEditor from "@uiw/react-md-editor";
import { getCommands, getExtraCommands } from "@uiw/react-md-editor/commands-cn";
import { useLanguage } from "@/lib/i18n";

const chineseCommands = getCommands();
const chineseExtraCommands = getExtraCommands();

export function LocalizedMarkdownEditor(props: ComponentProps<typeof MDEditor>) {
  const language = useLanguage();
  return language === "zh-CN"
    ? <MDEditor commands={chineseCommands} extraCommands={chineseExtraCommands} {...props} />
    : <MDEditor {...props} />;
}
