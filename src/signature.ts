export function signatureHtml(signature: string): string {
  if (/<\/?[a-z][^>]*>/i.test(signature)) return signature;
  return signature.split("\n").map((line) =>
    `<p>${line.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;")}</p>`).join("");
}
