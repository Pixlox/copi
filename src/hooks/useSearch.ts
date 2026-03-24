import { startTransition, useState, useCallback, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface ClipResult {
  id: number;
  content: string;
  content_type: string;
  source_app: string;
  created_at: number;
  pinned: boolean;
  source_app_icon: string | null;
  content_highlighted: string | null;
  ocr_text: string | null;
  image_thumbnail: string | null;
}

export type FilterType = "all" | "text" | "url" | "code" | "image" | "pinned";

export function useSearch() {
  const [query, setQuery] = useState("");
  const [activeFilter, setActiveFilter] = useState<FilterType>("all");
  const [results, setResults] = useState<ClipResult[]>([]);
  const [isSearching, setIsSearching] = useState(false);
  const [totalCount, setTotalCount] = useState(0);
  const requestIdRef = useRef(0);
  const resultsRef = useRef<ClipResult[]>([]);
  const totalCountRef = useRef(0);

  const applyResults = useCallback((nextResults: ClipResult[]) => {
    resultsRef.current = nextResults;
    startTransition(() => {
      setResults(nextResults);
    });
  }, []);

  const applyTotalCount = useCallback((nextTotalCount: number) => {
    totalCountRef.current = nextTotalCount;
    setTotalCount(nextTotalCount);
  }, []);

  useEffect(() => {
    resultsRef.current = results;
  }, [results]);

  useEffect(() => {
    totalCountRef.current = totalCount;
  }, [totalCount]);

  const fetchResults = useCallback(async (searchQuery: string, filter: FilterType) => {
    const requestId = ++requestIdRef.current;
    setIsSearching(true);
    try {
      const clips = await invoke<ClipResult[]>("search_clips", {
        query: searchQuery,
        filter,
      });
      if (requestId !== requestIdRef.current) {
        return;
      }
      applyResults(clips);
    } catch (error) {
      console.error("Search failed:", error);
      if (requestId !== requestIdRef.current) {
        return;
      }
      applyResults([]);
    } finally {
      if (requestId === requestIdRef.current) {
        setIsSearching(false);
      }
    }
  }, [applyResults]);

  const fetchCount = useCallback(async () => {
    try {
      const count = await invoke<number>("get_total_clip_count");
      applyTotalCount(count);
    } catch (error) {
      console.error("Failed to get clip count:", error);
    }
  }, [applyTotalCount]);

  // Store latest values in refs for the event listener
  const queryRef = useRef(query);
  const filterRef = useRef(activeFilter);
  queryRef.current = query;
  filterRef.current = activeFilter;

  // Debounced search
  useEffect(() => {
    const timer = setTimeout(() => {
      fetchResults(query, activeFilter);
    }, 80);

    return () => clearTimeout(timer);
  }, [query, activeFilter, fetchResults]);

  // Listen for new-clip events from clipboard watcher
  useEffect(() => {
    const refresh = () => {
      fetchResults(queryRef.current, filterRef.current);
      fetchCount();
    };

    const unlistenNew = listen("new-clip", refresh);
    const unlistenChanged = listen("clips-changed", refresh);

    return () => {
      unlistenNew.then((fn) => fn());
      unlistenChanged.then((fn) => fn());
    };
  }, [fetchResults, fetchCount]);

  // Listen for search-updated events from semantic search
  useEffect(() => {
    const unlisten = listen<ClipResult[]>("search-updated", (event) => {
      applyResults(event.payload);
    });

    return () => {
      unlisten.then((fn) => fn());
    };
  }, [applyResults]);

  // Fetch total count on mount
  useEffect(() => {
    fetchCount();
  }, [fetchCount]);

  const optimisticDelete = useCallback(
    (clipId: number) => {
      const previousResults = resultsRef.current;
      const previousCount = totalCountRef.current;
      const clipExists = previousResults.some((clip) => clip.id === clipId);
      const nextResults = previousResults.filter((clip) => clip.id !== clipId);

      requestIdRef.current += 1;
      applyResults(nextResults);
      if (clipExists) {
        applyTotalCount(Math.max(0, previousCount - 1));
      }

      return () => {
        requestIdRef.current += 1;
        applyResults(previousResults);
        applyTotalCount(previousCount);
      };
    },
    [applyResults, applyTotalCount]
  );

  const optimisticTogglePin = useCallback(
    (clipId: number) => {
      const previousResults = resultsRef.current;
      const nextResults = previousResults.flatMap((clip) => {
        if (clip.id !== clipId) {
          return [clip];
        }

        const nextClip = { ...clip, pinned: !clip.pinned };
        if (filterRef.current === "pinned" && !nextClip.pinned) {
          return [];
        }

        return [nextClip];
      });

      requestIdRef.current += 1;
      applyResults(nextResults);

      return () => {
        requestIdRef.current += 1;
        applyResults(previousResults);
      };
    },
    [applyResults]
  );

  return {
    query,
    setQuery,
    activeFilter,
    setActiveFilter,
    results,
    isSearching,
    totalCount,
    optimisticDelete,
    optimisticTogglePin,
    refresh: () => fetchResults(query, activeFilter),
  };
}
