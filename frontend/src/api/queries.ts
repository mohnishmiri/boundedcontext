import { keepPreviousData, useQuery } from '@tanstack/react-query';

import * as api from './client';
import type { JobStatus } from './types';

export const queryKeys = {
  me: ['auth', 'me'] as const,
  authNotice: ['auth', 'notice'] as const,
  agent: ['agent'] as const,
  models: ['models'] as const,
  organizations: ['github', 'organizations'] as const,
  repositories: (owner: string, q: string, page: number) => ['github', 'repositories', owner, q, page] as const,
  repository: (owner: string, repo: string) => ['github', 'repository', owner, repo] as const,
  branches: (owner: string, repo: string, q: string, page: number) =>
    ['github', 'branches', owner, repo, q, page] as const,
  jobs: ['jobs'] as const,
  jobList: (status: JobStatus | undefined, page: number) => ['jobs', 'list', status ?? 'all', page] as const,
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

export function useAgent(enabled = true) {
  return useQuery({
    queryKey: queryKeys.agent,
    queryFn: ({ signal }) => api.getAgent(signal),
    staleTime: 5 * 60_000,
    enabled,
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
