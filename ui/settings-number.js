// 整数入力の編集と正規化。範囲・既定値はバックエンドの宣言だけから受け取る。
(() => {
  const normalize = value => String(value ?? "").replace(/[０-９]/g, digit => String(digit.charCodeAt(0) - 0xff10)).replace(/－/g, "-").replace(/＋/g, "+").trim();
  const integer = value => /^[+-]?\d+$/.test(value);

  function reason(input, schema) {
    const value = normalize(input.value);
    if (!value || (schema.allow_auto && value === "auto")) return "";
    if (!integer(value)) return "整数を入力してください";
    const number = BigInt(value);
    if (Array.isArray(schema.range) && (number < BigInt(schema.range[0]) || number > BigInt(schema.range[1]))) return `${schema.range[0]}〜${schema.range[1]}${schema.unit || ""}の整数を入力してください`;
    return "";
  }

  function attach(input, schema) {
    input.dataset.integerSetting = "true";
    input.inputMode = "numeric";
    if (Array.isArray(schema.range)) {
      input.min = String(schema.range[0]);
      input.max = String(schema.range[1]);
    }
    input.step = "1";
    const restore = () => {
      const value = normalize(input.value);
      // numberは不正な草稿を空にするため、訂正前の文字はtextで保持する。
      input.type = !schema.allow_auto && (!value || integer(value)) ? "number" : "text";
      input.value = integer(value) ? BigInt(value).toString() : value;
    };
    input.addEventListener("focus", () => { if (!schema.allow_auto) input.type = "text"; });
    input.addEventListener("blur", restore);
    input.addEventListener("paste", event => {
      const pasted = event.clipboardData?.getData("text");
      if (typeof pasted !== "string") return;
      event.preventDefault();
      const start = input.selectionStart ?? input.value.length;
      const end = input.selectionEnd ?? input.value.length;
      if (input.type !== "text") input.type = "text";
      input.setRangeText(normalize(pasted), start, end, "end");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    input.addEventListener("compositionend", () => { input.value = normalize(input.value); });
    restore();
  }

  window.PicoIntegerInput = { normalize, reason, attach };
})();
