import type { RouteDescriptor } from "./common.js";
import { addressRouteDescriptors } from "../routes/addresses.js";
import { blockRouteDescriptors } from "../routes/blocks.js";
import { governanceSearchRouteDescriptors } from "../routes/governanceSearch.js";
import { liveRouteDescriptors } from "../routes/live.js";
import { metricsRouteDescriptors } from "../routes/metrics.js";
import { statusRawRouteDescriptors } from "../routes/statusRaw.js";
import { streamRouteDescriptors } from "../routes/stream.js";
import { transactionRouteDescriptors } from "../routes/transactions.js";
import { validatorRouteDescriptors } from "../routes/validators.js";

/**
 * One named route-descriptor group exposed to client consumers.
 */
export interface RouteDescriptorGroup {
  readonly [name: string]: RouteDescriptor;
}

/**
 * Public catalog of all route descriptors known to the package.
 */
export interface RouteCatalog {
  readonly addresses: RouteDescriptorGroup;
  readonly blocks: RouteDescriptorGroup;
  readonly governanceSearch: RouteDescriptorGroup;
  readonly live: RouteDescriptorGroup;
  readonly metrics: RouteDescriptorGroup;
  readonly statusRaw: RouteDescriptorGroup;
  readonly stream: RouteDescriptorGroup;
  readonly transactions: RouteDescriptorGroup;
  readonly validators: RouteDescriptorGroup;
}

/**
 * Stable public route catalog for package consumers.
 */
export const routeCatalog: RouteCatalog = {
  addresses: addressRouteDescriptors,
  blocks: blockRouteDescriptors,
  governanceSearch: governanceSearchRouteDescriptors,
  live: liveRouteDescriptors,
  metrics: metricsRouteDescriptors,
  statusRaw: statusRawRouteDescriptors,
  stream: streamRouteDescriptors,
  transactions: transactionRouteDescriptors,
  validators: validatorRouteDescriptors,
};
