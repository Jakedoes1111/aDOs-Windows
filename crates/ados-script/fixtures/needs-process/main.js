// Asks to run `date`. aDOs blocks it until the user says yes.
import { View, div } from "gpui-kit";
import { v_flex, Button } from "gpui-base";
import { run } from "process";

export default class Clock extends View {
  init() {
    this.out = "Press the button to run date.";
  }

  async date(cx) {
    // `cx` here is the AsyncContext from cx.spawn, valid across await.
    try {
      const result = await run("date", []);
      this.out = `date said: ${result.stdout.trim()}`;
    } catch (error) {
      this.out = `Could not run date: ${String(error?.message ?? error)}`;
    }
    cx.notify();
  }

  render(cx) {
    const colors = cx.theme().colors;
    return v_flex()
      .size_full()
      .p(16)
      .gap(12)
      .bg(colors.background)
      .text_color(colors.foreground)
      .child(
        Button.new("run-date")
          .h(32)
          .px(14)
          .items_center()
          .justify_center()
          .rounded(8)
          .bg(colors.primary)
          .text_color(colors.primary_foreground)
          .text_sm()
          .on_click((_event, cx) => { cx.spawn((acx) => this.date(acx)); })
          .child("Run date")
      )
      .child(div().text_sm().child(this.out));
  }
}
