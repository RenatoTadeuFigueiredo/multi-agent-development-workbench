// DDD role: ValueObject

package schemas

// #NonEmptyText prevents empty identifiers and routing explanations.
// DDD role: ValueObject
#NonEmptyText: {
    value: string & !=""
}

// #RoleId identifies a stable workflow role rather than a provider.
// DDD role: ValueObject
#RoleId: {
    value: #NonEmptyText
}

// #ModelAlias resolves independently from the workflow definition.
// DDD role: ValueObject
#ModelAlias: {
    value: #NonEmptyText
}

// #ProviderId selects an adapter registered with the daemon.
// DDD role: ValueObject
#ProviderId: {
    value: #NonEmptyText
}

// #PermissionScope summarizes the authority granted to one dispatch.
// DDD role: ValueObject
#PermissionScope: {
    mode: "read-only" | "approval-required" | "denied"
}

// #ReferenceSet is a first-class collection of tool or data-source references.
// DDD role: ValueObject
#ReferenceSet: {
    values: [...#NonEmptyText]
}

// #RouteDestination contains the independently configurable executor mapping.
// DDD role: ValueObject
#RouteDestination: {
    role:       #RoleId
    modelAlias: #ModelAlias
    provider:   #ProviderId
}

// #RouteContext contains the capabilities and inputs granted to the executor.
// DDD role: ValueObject
#RouteContext: {
    tools:       #ReferenceSet
    dataSources: #ReferenceSet
    permission:  #PermissionScope
}

// #RouteDecision explains why the destination was selected.
// DDD role: ValueObject
#RouteDecision: {
    risk:         "low" | "medium" | "high"
    confidence:   number & >=0 & <=1
    selectedRule: #NonEmptyText
}

// #RoutingPlan is the auditable result produced before provider dispatch.
// DDD role: ValueObject
#RoutingPlan: {
    intent:      #NonEmptyText
    destination: #RouteDestination
    context:     #RouteContext
    decision:    #RouteDecision
}
