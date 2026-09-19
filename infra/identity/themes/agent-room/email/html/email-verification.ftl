<#import "template.ftl" as layout>
<@layout.emailLayout>
    <@layout.actionEmail
        title=msg("arEmailVerifyTitle")
        greeting=(user.firstName?has_content)?then(msg("arEmailGreetingName", user.firstName), msg("arEmailGreeting"))
        body=msg("arEmailVerifyBody", realmName)
        button=msg("arEmailVerifyButton")
        link=link
        expiresIn=msg("arEmailExpires", linkExpirationFormatter(linkExpiration))
        ignore=msg("arEmailVerifyIgnore", realmName)/>
</@layout.emailLayout>
