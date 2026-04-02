import {
  ReadinessModel,
  RouteClass,
  type CursorPageParams,
  type LivePage,
  type RouteDescriptor,
} from "../contracts/common.js";
import type { ApiError } from "../contracts/errors.js";
import { type Result } from "../contracts/result.js";
import { failContract } from "../internal/errors.js";
import {
  expectBoolean,
  expectNumber,
  expectObject,
  expectOptionalString,
  expectString,
  parseLivePage,
  readField,
  readOptionalField,
} from "../internal/json.js";
import { encodePathSegment, createQuery } from "../internal/query.js";
import { requestJsonResult, type ClientContext } from "../internal/transport.js";

/**
 * Live governance proposal detail.
 */
export interface ProposalDetail {
  readonly hash: string;
  readonly collateralHash?: string;
  readonly name: string;
  readonly url?: string;
  readonly paymentAddress?: string;
  readonly paymentAmount: number;
  readonly startTime?: string;
  readonly endTime?: string;
  readonly yesCount: number;
  readonly noCount: number;
  readonly abstainCount: number;
  readonly voteCount: number;
  readonly totalNodes: number;
  readonly enabledNodes: number;
  readonly participatingNodes: number;
  readonly fundingThreshold: number;
  readonly funded: boolean;
  readonly valid: boolean;
  readonly creationTime?: string;
}

/**
 * Live governance vote summary.
 */
export interface ProposalVote {
  readonly proTxHash: string;
  readonly vote: string;
  readonly voteTime?: string;
}

/**
 * Search result variant for a matched block identifier.
 */
export interface SearchBlockResult {
  readonly type: "block";
  readonly id: string;
  readonly status: string;
  readonly blockTags: {
    readonly orphan: boolean;
  };
}

/**
 * Search result variant for a matched transaction identifier.
 */
export interface SearchTransactionResult {
  readonly type: "tx";
  readonly id: string;
  readonly txTags: {
    readonly mined: boolean;
    readonly coinbase: boolean;
  };
}

/**
 * Search result variant for a matched address identifier.
 */
export interface SearchAddressResult {
  readonly type: "address";
  readonly id: string;
}

/**
 * Search result variant for a matched validator identifier.
 */
export interface SearchValidatorResult {
  readonly type: "validator";
  readonly id: string;
  readonly status: string;
}

/**
 * Search result variant for a matched governance proposal identifier.
 */
export interface SearchProposalResult {
  readonly type: "proposal";
  readonly id: string;
}

/**
 * Discriminated global search result.
 */
export type SearchResult =
  | SearchAddressResult
  | SearchBlockResult
  | SearchProposalResult
  | SearchTransactionResult
  | SearchValidatorResult;

/**
 * Capped autocomplete-style global search results.
 */
export interface SearchList {
  readonly value: readonly SearchResult[];
}

/**
 * Route methods for live governance and cross-domain search.
 */
export interface GovernanceSearchRoutes {
  /**
   * Read one live page of current governance proposals.
   */
  listProposals(params?: CursorPageParams): Promise<Result<LivePage<ProposalDetail>, ApiError>>;

  /**
   * Read one live governance proposal by hash.
   */
  getProposal(hash: string): Promise<Result<ProposalDetail, ApiError>>;

  /**
   * Read one live page of votes for a proposal.
   */
  getProposalVotes(hash: string, params?: CursorPageParams): Promise<Result<LivePage<ProposalVote>, ApiError>>;

  /**
   * Resolve one global search query into a capped discriminated result list.
   */
  search(query: string): Promise<Result<SearchList, ApiError>>;
}

const PROPOSALS_ROUTE: RouteDescriptor = {
  method: "GET",
  name: "proposals",
  path: "/api/v1/proposals",
  readinessModel: ReadinessModel.LIVE,
  routeClass: RouteClass.LIVE,
};

const PROPOSAL_ROUTE: RouteDescriptor = {
  method: "GET",
  name: "proposal",
  path: "/api/v1/proposal/{hash}",
  readinessModel: ReadinessModel.LIVE,
  routeClass: RouteClass.LIVE,
};

const PROPOSAL_VOTES_ROUTE: RouteDescriptor = {
  method: "GET",
  name: "proposalVotes",
  path: "/api/v1/proposal/{hash}/votes",
  readinessModel: ReadinessModel.LIVE,
  routeClass: RouteClass.LIVE,
};

const SEARCH_ROUTE: RouteDescriptor = {
  method: "GET",
  name: "search",
  path: "/api/v1/search",
  readinessModel: ReadinessModel.NONE,
  routeClass: RouteClass.CROSS_DOMAIN,
};

/**
 * Public route descriptors for governance and search routes.
 */
export const governanceSearchRouteDescriptors = {
  proposal: PROPOSAL_ROUTE,
  proposalVotes: PROPOSAL_VOTES_ROUTE,
  proposals: PROPOSALS_ROUTE,
  search: SEARCH_ROUTE,
} as const;

function parseProposalDetail(value: unknown, path: string): ProposalDetail {
  const object = expectObject(value, path);
  const collateralHash = expectOptionalString(readOptionalField(object, "collateralHash"), `${path}.collateralHash`);
  const url = expectOptionalString(readOptionalField(object, "url"), `${path}.url`);
  const paymentAddress = expectOptionalString(readOptionalField(object, "paymentAddress"), `${path}.paymentAddress`);
  const startTime = expectOptionalString(readOptionalField(object, "startTime"), `${path}.startTime`);
  const endTime = expectOptionalString(readOptionalField(object, "endTime"), `${path}.endTime`);
  const creationTime = expectOptionalString(readOptionalField(object, "creationTime"), `${path}.creationTime`);
  return {
    hash: expectString(readField(object, "hash", path), `${path}.hash`),
    ...(collateralHash == null ? {} : { collateralHash }),
    name: expectString(readField(object, "name", path), `${path}.name`),
    ...(url == null ? {} : { url }),
    ...(paymentAddress == null ? {} : { paymentAddress }),
    paymentAmount: expectNumber(readField(object, "paymentAmount", path), `${path}.paymentAmount`),
    ...(startTime == null ? {} : { startTime }),
    ...(endTime == null ? {} : { endTime }),
    yesCount: expectNumber(readField(object, "yesCount", path), `${path}.yesCount`),
    noCount: expectNumber(readField(object, "noCount", path), `${path}.noCount`),
    abstainCount: expectNumber(readField(object, "abstainCount", path), `${path}.abstainCount`),
    voteCount: expectNumber(readField(object, "voteCount", path), `${path}.voteCount`),
    totalNodes: expectNumber(readField(object, "totalNodes", path), `${path}.totalNodes`),
    enabledNodes: expectNumber(readField(object, "enabledNodes", path), `${path}.enabledNodes`),
    participatingNodes: expectNumber(readField(object, "participatingNodes", path), `${path}.participatingNodes`),
    fundingThreshold: expectNumber(readField(object, "fundingThreshold", path), `${path}.fundingThreshold`),
    funded: expectBoolean(readField(object, "funded", path), `${path}.funded`),
    valid: expectBoolean(readField(object, "valid", path), `${path}.valid`),
    ...(creationTime == null ? {} : { creationTime }),
  };
}

function parseProposalVote(value: unknown, path: string): ProposalVote {
  const object = expectObject(value, path);
  const voteTime = expectOptionalString(readOptionalField(object, "voteTime"), `${path}.voteTime`);
  return {
    proTxHash: expectString(readField(object, "proTxHash", path), `${path}.proTxHash`),
    vote: expectString(readField(object, "vote", path), `${path}.vote`),
    ...(voteTime == null ? {} : { voteTime }),
  };
}

function parseSearchResult(value: unknown, path: string): SearchResult {
  const object = expectObject(value, path);
  const kind = expectString(readField(object, "type", path), `${path}.type`);
  switch (kind) {
    case "address":
      return {
        type: "address",
        id: expectString(readField(object, "id", path), `${path}.id`),
      };
    case "block": {
      const blockTags = expectObject(readField(object, "blockTags", path), `${path}.blockTags`);
      return {
        type: "block",
        id: expectString(readField(object, "id", path), `${path}.id`),
        status: expectString(readField(object, "status", path), `${path}.status`),
        blockTags: {
          orphan: expectBoolean(readField(blockTags, "orphan", `${path}.blockTags`), `${path}.blockTags.orphan`),
        },
      };
    }
    case "proposal":
      return {
        type: "proposal",
        id: expectString(readField(object, "id", path), `${path}.id`),
      };
    case "tx": {
      const txTags = expectObject(readField(object, "txTags", path), `${path}.txTags`);
      return {
        type: "tx",
        id: expectString(readField(object, "id", path), `${path}.id`),
        txTags: {
          mined: expectBoolean(readField(txTags, "mined", `${path}.txTags`), `${path}.txTags.mined`),
          coinbase: expectBoolean(readField(txTags, "coinbase", `${path}.txTags`), `${path}.txTags.coinbase`),
        },
      };
    }
    case "validator":
      return {
        type: "validator",
        id: expectString(readField(object, "id", path), `${path}.id`),
        status: expectString(readField(object, "status", path), `${path}.status`),
      };
    default:
      return failContract(`${path}.type`, `unsupported search result type ${kind}`);
  }
}

function parseSearchList(value: unknown, path: string): SearchList {
  return {
    value: parseLivePage(value, path, parseSearchResult).value,
  };
}

function createCursorQuery(params?: CursorPageParams): URLSearchParams | undefined {
  if (params == null) {
    return undefined;
  }
  const query = createQuery({
    cursor: params.cursor,
    limit: params.limit,
  });
  return query.size > 0 ? query : undefined;
}

/**
 * Build the governance and search route family for one client transport context.
 */
export function buildGovernanceSearchRoutes(context: ClientContext): GovernanceSearchRoutes {
  return {
    async getProposal(hash: string): Promise<Result<ProposalDetail, ApiError>> {
      return requestJsonResult(
        context,
        PROPOSAL_ROUTE,
        `/api/v1/proposal/${encodePathSegment(hash)}`,
        parseProposalDetail
      );
    },

    async getProposalVotes(hash: string, params?: CursorPageParams): Promise<Result<LivePage<ProposalVote>, ApiError>> {
      const query = createCursorQuery(params);
      return requestJsonResult(
        context,
        PROPOSAL_VOTES_ROUTE,
        `/api/v1/proposal/${encodePathSegment(hash)}/votes`,
        (value: unknown, path: string) => parseLivePage(value, path, parseProposalVote),
        ...(query == null ? [] : [{ query }])
      );
    },

    async listProposals(params?: CursorPageParams): Promise<Result<LivePage<ProposalDetail>, ApiError>> {
      const query = createCursorQuery(params);
      return requestJsonResult(
        context,
        PROPOSALS_ROUTE,
        PROPOSALS_ROUTE.path,
        (value: unknown, path: string) => parseLivePage(value, path, parseProposalDetail),
        ...(query == null ? [] : [{ query }])
      );
    },

    async search(query: string): Promise<Result<SearchList, ApiError>> {
      const params = createQuery({ q: query });
      return requestJsonResult(context, SEARCH_ROUTE, SEARCH_ROUTE.path, parseSearchList, { query: params });
    },
  };
}
