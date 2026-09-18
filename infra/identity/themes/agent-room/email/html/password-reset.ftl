<#import "template.ftl" as layout>
<@layout.emailLayout>
    <@layout.actionEmail
        title=msg("arEmailResetTitle")
        greeting=(user.firstName?has_content)?then(msg("arEmailGreetingName", user.firstName), msg("arEmailGreeting"))
        body=msg("arEmailResetBody", realmName)
        button=msg("arEmailResetButton")
        link=link
        expiresIn=msg("arEmailExpires", linkExpirationFormatter(linkExpiration))
        ignore=msg("arEmailResetIgnore")/>
</@layout.emailLayout>
