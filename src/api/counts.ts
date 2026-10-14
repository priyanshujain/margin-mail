import { call, type Place, type PlaceCounts } from "../ipc";

export const placeCounts = () => call<PlaceCounts[]>("place_counts");

export const placeVisited = (accountId: string | null, place: Place) =>
  call<null>("place_visited", { accountId, place });
