import type { QueryClient } from '@tanstack/react-query';
import { QueryClientProvider } from '@tanstack/react-query';
import { render } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { ReactElement } from 'react';
import { MemoryRouter, useLocation } from 'react-router-dom';

import { AppRoutes } from '../App';
import { createQueryClient } from '../lib/queryClient';
import { ROUTER_FUTURE } from '../lib/router';

function LocationProbe() {
  const location = useLocation();
  return <div data-testid="location">{`${location.pathname}${location.search}`}</div>;
}

export function renderWithProviders(ui: ReactElement, opts: { route?: string; queryClient?: QueryClient } = {}) {
  const queryClient = opts.queryClient ?? createQueryClient({ retry: false });
  const result = render(
    <QueryClientProvider client={queryClient}>
      <MemoryRouter initialEntries={[opts.route ?? '/']} future={ROUTER_FUTURE}>
        {ui}
        <LocationProbe />
      </MemoryRouter>
    </QueryClientProvider>,
  );
  return { ...result, queryClient, user: userEvent.setup() };
}

export function renderApp(route = '/', opts: { queryClient?: QueryClient } = {}) {
  return renderWithProviders(<AppRoutes />, { route, ...opts });
}
