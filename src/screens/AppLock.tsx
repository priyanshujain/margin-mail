import { useEffect, useState, type ReactNode } from "react";
import { appLockStatus, appUnlock, type AppLockStatus } from "../api/appLock";
import { Button } from "../ui/Button";
import "./app-lock.css";

export function AppLockGate({ children }: { children: ReactNode }) {
  const [status, setStatus] = useState<AppLockStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const readStatus = async () => {
    setBusy(true);
    setError(null);
    try {
      setStatus(await appLockStatus());
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  useEffect(() => {
    void readStatus();
  }, []);

  const unlock = async () => {
    setBusy(true);
    setError(null);
    try {
      await appUnlock();
      setStatus(await appLockStatus());
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  if (status && !status.locked) return children;
  if (!status && !error) return <div className="app" aria-busy="true" />;

  return (
    <div className="app app-lock">
      <main className="app-lock-content">
        <h1 className="app-lock-title">Mailbox locked</h1>
        <p className="app-lock-description">
          Use Touch ID or your Mac login password to open your mailbox.
        </p>
        {error || status?.unavailableReason ? (
          <p className="app-lock-error" role="alert">{error ?? status?.unavailableReason}</p>
        ) : null}
        <Button
          variant="primary"
          onClick={() => void (status?.available ? unlock() : readStatus())}
          disabled={busy}
        >
          {busy ? "Waiting for macOS…" : status?.available ? "Unlock mailbox" : "Try again"}
        </Button>
      </main>
    </div>
  );
}
