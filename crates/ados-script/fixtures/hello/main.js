// Sample aDOs script panel: a draft kept in panel storage, a button that runs an
// aDOs action, and the live context of whatever module is active.
import { View, div } from "gpui-kit";
import { h_flex, v_flex, Button, Input, InputState } from "gpui-base";
import { context, run_action, storage_get, storage_set } from "ados";

const VERSION = "v1";

export default class Hello extends View {
  init() {
    // The draft lives in ados storage, so a reload or relaunch keeps it.
    this.draft = InputState.new({ placeholder: "Write a draft", value: storage_get("draft") ?? "" });
    this.draft.on("change", () => storage_set("draft", this.draft.value()));
    this.status = "";
  }

  ping(cx) {
    try {
      run_action("demo.ping");
      this.status = "Ran demo.ping.";
    } catch (error) {
      this.status = String(error?.message ?? error);
    }
    cx.notify();
  }

  render(cx) {
    const colors = cx.theme().colors;
    const ctx = context();
    return v_flex()
      .size_full()
      .p(16)
      .gap(12)
      .bg(colors.background)
      .text_color(colors.foreground)
      .child(div().text_sm().text_color(colors.muted_foreground).child(`Hello panel ${VERSION}`))
      .child(
        Input.new(this.draft)
          .h(32)
          .px(8)
          .border(1)
          .border_color(colors.border)
          .rounded(8)
      )
      .child(
        h_flex()
          .gap(8)
          .items_center()
          .child(
            Button.new("ping")
              .h(32)
              .px(14)
              .items_center()
              .justify_center()
              .rounded(8)
              .bg(colors.primary)
              .text_color(colors.primary_foreground)
              .text_sm()
              .on_click((_event, cx) => this.ping(cx))
              .child("Run demo.ping")
          )
          .child(div().text_sm().text_color(colors.muted_foreground).child(this.status))
      )
      .child(
        div()
          .text_sm()
          .child(ctx ? `Context: ${ctx.title} (${ctx.module}): ${ctx.text}` : "Context: nothing active")
      );
  }
}
