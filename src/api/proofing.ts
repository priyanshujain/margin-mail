import { call } from "../ipc";

export interface ProofIssue {
  start: number;
  end: number;
  kind: "spelling" | "grammar";
  message: string;
  suggestions: string[];
}

export const proofText = (text: string, spelling: boolean, grammar: boolean) =>
  call<ProofIssue[]>("proof_text", { text, spelling, grammar });
