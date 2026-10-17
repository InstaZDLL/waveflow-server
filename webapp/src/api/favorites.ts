import { call } from "./client";

export type Favorite = {
  entity_type: string;
  entity_id: string;
  starred_at: number;
};

export const listFavorites = () => call<Favorite[]>("/api/v2/favorites");

export const setFavorite = (kind: string, id: string, on: boolean) =>
  call<void>(`/api/v2/favorites/${kind}/${id}`, {
    method: on ? "PUT" : "DELETE",
  });

/** The three things a star, a rating or a bookmark can hang off. */
export type EntityKind = "track" | "album" | "artist";

export type Rating = {
  entity_type: string;
  entity_id: string;
  rating: number;
  updated_at: number;
};

export const listRatings = () => call<Rating[]>("/api/v2/ratings");

/** 1 to 5 stars; 0 clears the rating, which is the server's own convention. */
export const setRating = (kind: EntityKind, id: string, rating: number) =>
  call<void>(`/api/v2/ratings/${kind}/${id}`, {
    method: "PUT",
    body: JSON.stringify({ rating }),
  });
