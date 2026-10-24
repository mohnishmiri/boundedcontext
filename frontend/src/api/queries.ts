import { keepPreviousData, useQueries, useQuery } from '@tanstack/react-query';

import * as api from './client';
import { isTerminalStatus } from '../lib/jobState';
import type { JobStatus } from './types';

export const queryKeys = {
  me: ['auth', 'me'] as const,
  authNotice: ['auth', 'notice'] as const,
  generators: ['generators'] as const,
  generatorAgent: (id: string) => ['generators', 'agent', id] as const,
  models: ['models'] as const,
  organizations: ['github', 'organizations'] as const,
  repositories: (owner: string, q: string, page: number) => ['github', 'repositories', owner, q, page] as const,
  repository: (owner: string, repo: string) => ['github', 'repository', owner, repo] as const,
  branches: (owner: string, repo: string, q: string, page: number) =>
    ['github', 'branches', owner, repo, q, page] as const,
  jobs: ['jobs'] as const,
  jobList: (status: JobStatus | undefined, page: number) => ['jobs', 'list', status ?? 'all', page] as const,
  batchJobs: (batchId: string) => ['jobs', 'batch', batchId] as const,
  job: (id: string) => ['jobs', 'detail', id] as const,
  jobDiff: (id: string) => ['jobs', 'diff', id] as const,
};

export function useMe() {
  return useQuery({
    queryKey: queryKeys.me,
    queryFn: ({ signal }) => api.getMe(signal),
    staleTime: 5 * 60_000,
  });
}

export function useGenerators() {
  return useQuery({
    queryKey: queryKeys.generators,
    queryFn: ({ signal }) => api.listGenerators(signal),
    staleTime: 60_000,
  });
}

export function useGeneratorAgent(generatorId: string | null) {
  return useQuery({
    queryKey: queryKeys.generatorAgent(generatorId ?? ''),
    queryFn: ({ signal }) => api.getGeneratorAgent(generatorId ?? '', signal),
    enabled: Boolean(generatorId),
    staleTime: 5 * 60_000,
  });
}

export function useModels() {
  return useQuery({
    queryKey: queryKeys.models,
    queryFn: ({ signal }) => api.getModels(signal),
    staleTime: 5 * 60_000,
  });
}

export function useOrganizations() {
  return useQuery({
    queryKey: queryKeys.organizations,
    queryFn: ({ signal }) => api.listOrganizations(signal),
    staleTime: 60_000,
  });
}

export const REPOS_PER_PAGE = 30;
export const BRANCHES_PER_PAGE = 50;

export function useRepositories(owner: string | null, q: string, page: number) {
  return useQuery({
    queryKey: queryKeys.repositories(owner ?? '', q, page),
    queryFn: ({ signal }) =>
      api.listRepositories({ owner: owner ?? '', q: q || undefined, page, per_page: REPOS_PER_PAGE }, signal),
    enabled: Boolean(owner),
    placeholderData: keepPreviousData,
    staleTime: 60_000,
  });
}

export function useRepository(owner: string | null, repo: string | null) {
  return useQuery({
    queryKey: queryKeys.repository(owner ?? '', repo ?? ''),
    queryFn: ({ signal }) => api.getRepository(owner ?? '', repo ?? '', signal),
    enabled: Boolean(owner && repo),
    staleTime: 60_000,
  });
}

/** Access details for several repositories at once; shares cache entries with `useRepository`. */
export function useRepositoryDetails(repos: { owner: string; repo: string }[]) {
  return useQueries({
    queries: repos.map((r) => ({
      queryKey: queryKeys.repository(r.owner, r.repo),
      queryFn: ({ signal }: { signal: AbortSignal }) => api.getRepository(r.owner, r.repo, signal),
      staleTime: 60_000,
    })),
  });
}

export function useBranches(owner: string | null, repo: string | null, q: string, page: number) {
  return useQuery({
    queryKey: queryKeys.branches(owner ?? '', repo ?? '', q, page),
    queryFn: ({ signal }) =>
      api.listBranches(owner ?? '', repo ?? '', { q: q || undefined, page, per_page: BRANCHES_PER_PAGE }, signal),
    enabled: Boolean(owner && repo),
    placeholderData: keepPreviousData,
    staleTime: 60_000,
  });
}

export const JOBS_PER_PAGE = 20;

export function useJobList(status: JobStatus | undefined, page: number) {
  return useQuery({
    queryKey: queryKeys.jobList(status, page),
    queryFn: ({ signal }) => api.listJobs({ status, page, per_page: JOBS_PER_PAGE }, signal),
    placeholderData: keepPreviousData,
    refetchInterval: (query) => {
      const items = query.state.data?.items ?? [];
      const active = items.some(
        (j) => j.status === 'queued' || j.status === 'running' || j.publish_status === 'queued' || j.publish_status === 'running',
      );
      return active ? 5_000 : false;
    },
  });
}

export function useJobDiff(id: string, enabled: boolean) {
  return useQuery({
    queryKey: queryKeys.jobDiff(id),
    queryFn: ({ signal }) => api.getJobDiff(id, signal),
    enabled,
    staleTime: Infinity,
  });
}

export const BATCH_JOBS_PER_PAGE = 100;
export const BATCH_POLL_MS = 3_000;

/** The jobs of one batch, polled until every job has reached a terminal status. */
export function useBatchJobs(batchId: string) {
  return useQuery({
    queryKey: queryKeys.batchJobs(batchId),
    queryFn: ({ signal }) => api.listJobs({ batch_id: batchId, per_page: BATCH_JOBS_PER_PAGE }, signal),
    refetchInterval: (query) => {
      const items = query.state.data?.items;
      if (!items) return false;
      return items.some((j) => !isTerminalStatus(j.status)) ? BATCH_POLL_MS : false;
    },
  });
}
