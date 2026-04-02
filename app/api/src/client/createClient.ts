import { routeCatalog, type RouteCatalog } from "../contracts/catalog.js";
import type { ClientOptions } from "../contracts/common.js";
import { createClientContext } from "../internal/transport.js";
import { buildAddressRoutes, type AddressRoutes } from "../routes/addresses.js";
import { buildBlockRoutes, type BlockRoutes } from "../routes/blocks.js";
import { buildGovernanceSearchRoutes, type GovernanceSearchRoutes } from "../routes/governanceSearch.js";
import { buildLiveRoutes, type LiveRoutes } from "../routes/live.js";
import { buildMetricsRoutes, type MetricsRoutes } from "../routes/metrics.js";
import { buildRawRoutes, type RawRoutes, buildStatusRoutes, type StatusRoutes } from "../routes/statusRaw.js";
import { buildStreamRoutes, type StreamRoutes } from "../routes/stream.js";
import { buildTransactionRoutes, type TransactionRoutes } from "../routes/transactions.js";
import { buildValidatorRoutes, type ValidatorRoutes } from "../routes/validators.js";

/**
 * Public Hyphen API client surface.
 */
export interface Client {
  readonly addresses: AddressRoutes;
  readonly blocks: BlockRoutes;
  readonly descriptors: RouteCatalog;
  readonly governanceSearch: GovernanceSearchRoutes;
  readonly raw: RawRoutes;
  readonly status: StatusRoutes;
  readonly stream: StreamRoutes;
  readonly transactions: TransactionRoutes;
  readonly live: LiveRoutes;
  readonly metrics: MetricsRoutes;
  readonly validators: ValidatorRoutes;
}

/**
 * Create one typed Hyphen API client bound to a daemon base URL.
 */
export function createClient(options: ClientOptions): Client {
  const context = createClientContext(options);
  return {
    addresses: buildAddressRoutes(context),
    blocks: buildBlockRoutes(context),
    descriptors: routeCatalog,
    governanceSearch: buildGovernanceSearchRoutes(context),
    live: buildLiveRoutes(context),
    metrics: buildMetricsRoutes(context),
    raw: buildRawRoutes(context),
    status: buildStatusRoutes(context),
    stream: buildStreamRoutes(context),
    transactions: buildTransactionRoutes(context),
    validators: buildValidatorRoutes(context),
  };
}
