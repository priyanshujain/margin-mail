/**
 * What a webview has to be told before it stops offering to fill a field in from history.
 *
 * An `<input type="text">` with no `autocomplete` is a form field as far as WebKit is concerned, so
 * it keeps what was typed into one and offers it back later as a pill with a cross on it, sitting
 * under the field and looking for all the world like something this app is suggesting. In the
 * palette that is worse than untidy: the palette's whole promise is that every row in it is a
 * command that exists, and a row that came from the browser's memory of last Tuesday is not.
 *
 * The other three go with it. A command, a search query, a host name and a port are not prose, and
 * autocorrect on any of them is a field that quietly changes what you typed.
 */
export const NO_AUTOFILL = {
  autoComplete: "off",
  autoCorrect: "off",
  autoCapitalize: "off",
  spellCheck: false,
} as const;
