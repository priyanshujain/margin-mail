// The `?` sheet, generated from the binding table. There is no list of shortcuts anywhere in this
// file, which is the entire point: a binding that exists is documented, and one that is removed
// stops being documented, without anybody remembering to do either.
//
// It takes `open` and `onClose` as props rather than reading the overlay store, so this module
// stays a function of the table and nothing else.

import { Key, Sheet } from "../ui";
import { BINDINGS, GROUPS, keyLabel } from "./bindings";
import "./shortcuts.css";

export interface ShortcutsSheetProps {
  open: boolean;
  onClose: () => void;
}

export function ShortcutsSheet({ open, onClose }: ShortcutsSheetProps) {
  return (
    <Sheet open={open} title="Keyboard shortcuts" size="wide" onClose={onClose}>
      <div className="shortcuts">
        {GROUPS.map((group) => {
          // A binding with no keys is a palette row rather than a shortcut, and a sheet of
          // shortcuts that lists one with no keycap beside it is a sheet that has lost the plot.
          const rows = BINDINGS.filter((binding) => binding.group === group && binding.keys.length > 0);
          if (rows.length === 0) return null;
          return (
            <section className="shortcuts-group" key={group}>
              <h3 className="shortcuts-heading">{group}</h3>
              <dl className="shortcuts-list">
                {rows.map((binding) => (
                  <div className="shortcuts-row" key={`${binding.context}:${binding.keys.join(" ")}`}>
                    <dt className="shortcuts-keys">
                      {binding.keys.map((key) => (
                        <Key key={key}>{keyLabel(key)}</Key>
                      ))}
                    </dt>
                    <dd className="shortcuts-label">{binding.label}</dd>
                  </div>
                ))}
              </dl>
            </section>
          );
        })}
      </div>
      <p className="shortcuts-note">
        Nothing is modal and nothing is chorded. Keys stand back while a text field has the focus,
        except the palette, which is how you get out of anywhere.
      </p>
    </Sheet>
  );
}

export default ShortcutsSheet;
